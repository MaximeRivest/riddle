package main

import (
	"context"
	"crypto"
	"crypto/rand"
	"crypto/rsa"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"io"
	"math/big"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

type authRoundTripper func(*http.Request) (*http.Response, error)

func (f authRoundTripper) RoundTrip(r *http.Request) (*http.Response, error) { return f(r) }
func mockAuth(t *testing.T, fn authRoundTripper) {
	t.Helper()
	old := authHTTP
	authHTTP = &http.Client{Transport: fn}
	t.Cleanup(func() { authHTTP = old })
}
func jsonAuthResponse(v interface{}) *http.Response {
	b, _ := json.Marshal(v)
	return &http.Response{StatusCode: 200, Header: make(http.Header), Body: io.NopCloser(strings.NewReader(string(b)))}
}

func TestPKCE(t *testing.T) {
	if got := pkce("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"); got != "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM" {
		t.Fatal(got)
	}
	s, e := randomString()
	if e != nil || len(s) != 43 || strings.Contains(s, "=") {
		t.Fatal("invalid random verifier")
	}
}
func TestCallbackStateAndRegistration(t *testing.T) {
	valid := url.Values{"state": {"expected"}, "code": {"code"}, "client_id": {"oaiapp_test"}}
	if _, e := parseCallback(valid, "expected", "dynamic_agent_client"); e != nil {
		t.Fatal(e)
	}
	for _, q := range []url.Values{{"state": {"wrong"}, "code": {"code"}, "client_id": {"oaiapp_test"}}, {"state": {"expected", "expected"}, "code": {"code"}, "client_id": {"oaiapp_test"}}, {"state": {"expected"}, "code": {"code"}}, {"state": {"expected"}, "code": {"code"}, "client_id": {"dynamic_agent_client"}}} {
		if _, e := parseCallback(q, "expected", "dynamic_agent_client"); e == nil {
			t.Fatal("invalid callback accepted")
		}
	}
	if _, e := parseCallback(valid, "expected", "other-client"); e == nil {
		t.Fatal("client switch accepted")
	}
	declined, e := parseCallback(url.Values{"state": {"expected"}, "error": {"access_denied"}}, "expected", "dynamic_agent_client")
	if e != nil || declined.err == nil {
		t.Fatal("decline not reported")
	}
}
func TestClaims(t *testing.T) {
	base := identityClaims{Issuer: authIssuer, Audience: json.RawMessage(`"client"`), Subject: "subject", Nonce: "nonce", Expires: 200}
	if err := validateClaims(base, "client", "nonce", 100); err != nil {
		t.Fatal(err)
	}
	cases := []identityClaims{base, base, base, base, base, base}
	cases[0].Issuer = "https://attacker.test"
	cases[1].Audience = json.RawMessage(`"wrong"`)
	cases[2].Expires = 100
	cases[3].Subject = ""
	cases[4].Nonce = "other"
	cases[5].Audience = json.RawMessage(`["client","other"]`)
	for _, c := range cases {
		if validateClaims(c, "client", "nonce", 100) == nil {
			t.Fatal("invalid claims accepted")
		}
	}
	multi := base
	multi.Audience = json.RawMessage(`["client","other"]`)
	multi.AuthorizedParty = "client"
	if e := validateClaims(multi, "client", "nonce", 100); e != nil {
		t.Fatal(e)
	}
}
func TestCredentialPersistence(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "state")
	c := Credentials{ClientID: "client", Subject: "sub", AccessToken: "secret", Scope: requestedScopes}
	if err := saveCredentials(dir, c); err != nil {
		t.Fatal(err)
	}
	loaded, err := loadCredentials(dir)
	if err != nil || loaded != c {
		t.Fatal("credential roundtrip failed")
	}
	info, _ := os.Stat(filepath.Join(dir, "credentials.json"))
	if info.Mode().Perm() != 0600 {
		t.Fatal("credentials not private")
	}
	info, _ = os.Stat(dir)
	if info.Mode().Perm() != 0700 {
		t.Fatal("directory not private")
	}
	first, err := hostID(dir)
	if err != nil {
		t.Fatal(err)
	}
	second, err := hostID(dir)
	if err != nil || first != second || !strings.HasPrefix(first, "urn:uuid:") {
		t.Fatal("host identity changed")
	}
}
func TestRefreshRotationAndScope(t *testing.T) {
	dir := t.TempDir()
	old := Credentials{ClientID: "client", Subject: "subject", RefreshToken: "old-refresh", Scope: requestedScopes}
	if e := saveCredentials(dir, old); e != nil {
		t.Fatal(e)
	}
	calls := 0
	mockAuth(t, func(r *http.Request) (*http.Response, error) {
		calls++
		if r.URL.String() != authIssuer+"/api/accounts/oauth/token" || r.Method != "POST" {
			t.Fatal("wrong token endpoint")
		}
		if e := r.ParseForm(); e != nil {
			t.Fatal(e)
		}
		if r.Form.Get("scope") != "" || r.Form.Get("client_id") != "client" || r.Form.Get("refresh_token") != "old-refresh" || r.Form.Get("resource") != authResource || r.Form.Get("grant_type") != "refresh_token" {
			t.Fatal("incorrect refresh request")
		}
		return jsonAuthResponse(tokenResponse{AccessToken: "fresh", RefreshToken: "new-refresh", ExpiresIn: 3600, TokenType: "Bearer"}), nil
	})
	token, err := accessToken(dir)
	if err != nil || token != "fresh" {
		t.Fatal("refresh failed", err)
	}
	saved, _ := loadCredentials(dir)
	if saved.RefreshToken != "new-refresh" || saved.Scope != requestedScopes || saved.ExpiresAt <= time.Now().Unix() {
		t.Fatal("rotated credential not persisted")
	}
	if _, err = accessToken(dir); err != nil || calls != 1 {
		t.Fatal("unexpired token unexpectedly refreshed")
	}
	saved.Scope = "openid"
	saveCredentials(dir, saved)
	if _, err = accessToken(dir); err == nil || calls != 1 {
		t.Fatal("missing plan permission accepted")
	}
	if requirePlanScope("chatgpt.tokens.use.direct.fake") == nil {
		t.Fatal("scope substring accepted")
	}
}
func signedIDToken(t *testing.T, key *rsa.PrivateKey, claims identityClaims) string {
	t.Helper()
	h, _ := json.Marshal(map[string]string{"alg": "RS256", "kid": "test-key"})
	b, _ := json.Marshal(claims)
	unsigned := base64.RawURLEncoding.EncodeToString(h) + "." + base64.RawURLEncoding.EncodeToString(b)
	digest := sha256.Sum256([]byte(unsigned))
	sig, err := rsa.SignPKCS1v15(rand.Reader, key, crypto.SHA256, digest[:])
	if err != nil {
		t.Fatal(err)
	}
	return unsigned + "." + base64.RawURLEncoding.EncodeToString(sig)
}
func TestIDTokenSignatureVerification(t *testing.T) {
	key, err := rsa.GenerateKey(rand.Reader, 2048)
	if err != nil {
		t.Fatal(err)
	}
	claims := identityClaims{Issuer: authIssuer, Audience: json.RawMessage(`"client"`), Subject: "subject", Nonce: "nonce", Expires: time.Now().Unix() + 300}
	mockAuth(t, func(r *http.Request) (*http.Response, error) {
		switch r.URL.Path {
		case "/.well-known/openid-configuration":
			return jsonAuthResponse(map[string]string{"issuer": authIssuer, "jwks_uri": authIssuer + "/.well-known/jwks.json"}), nil
		case "/.well-known/jwks.json":
			return jsonAuthResponse(map[string]interface{}{"keys": []map[string]string{{"kid": "test-key", "kty": "RSA", "alg": "RS256", "use": "sig", "n": base64.RawURLEncoding.EncodeToString(key.N.Bytes()), "e": base64.RawURLEncoding.EncodeToString(big.NewInt(int64(key.E)).Bytes())}}}), nil
		default:
			t.Fatal("unexpected endpoint")
			return nil, nil
		}
	})
	token := signedIDToken(t, key, claims)
	identity, err := verifyIdentity(context.Background(), token, "client", "nonce")
	if err != nil || identity.Subject != "subject" {
		t.Fatal("valid signature rejected", err)
	}
	parts := strings.Split(token, ".")
	parts[1] = base64.RawURLEncoding.EncodeToString([]byte(`{"iss":"https://auth.openai.com","aud":"client","sub":"attacker","exp":9999999999,"nonce":"nonce"}`))
	if _, err = verifyIdentity(context.Background(), strings.Join(parts, "."), "client", "nonce"); err == nil {
		t.Fatal("forged claims accepted")
	}
	if _, err = verifyIdentity(context.Background(), token, "client", "wrong"); err == nil {
		t.Fatal("wrong nonce accepted")
	}
	parts[0] = base64.RawURLEncoding.EncodeToString([]byte(`{"alg":"none","kid":"test-key"}`))
	if _, err = verifyIdentity(context.Background(), strings.Join(parts, "."), "client", "nonce"); err == nil {
		t.Fatal("unsigned token accepted")
	}
}
