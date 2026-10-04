package main

import (
	"context"
	"crypto"
	"crypto/rand"
	"crypto/rsa"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"math/big"
	"net"
	"net/http"
	"net/url"
	"os"
	"os/signal"
	"path/filepath"
	"strings"
	"syscall"
	"time"
)

const authIssuer = "https://auth.openai.com"
const authResource = "https://api.openai.com/v1"
const requestedScopes = "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct"

var authHTTP = &http.Client{Timeout: 30 * time.Second, CheckRedirect: func(req *http.Request, via []*http.Request) error { return http.ErrUseLastResponse }}

type Credentials struct {
	ClientID     string `json:"client_id"`
	HostID       string `json:"ext_agent_host_id"`
	Subject      string `json:"subject"`
	Email        string `json:"email"`
	IDToken      string `json:"id_token"`
	AccessToken  string `json:"access_token"`
	RefreshToken string `json:"refresh_token"`
	Scope        string `json:"scope"`
	ExpiresAt    int64  `json:"expires_at"`
}

func loadCredentials(dir string) (Credentials, error) {
	var c Credentials
	b, err := os.ReadFile(filepath.Join(dir, "credentials.json"))
	if err != nil {
		return c, err
	}
	err = json.Unmarshal(b, &c)
	return c, err
}
func privateWrite(path string, b []byte) error {
	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, 0700); err != nil {
		return err
	}
	if err := os.Chmod(dir, 0700); err != nil {
		return err
	}
	f, err := os.CreateTemp(dir, ".credentials-*")
	if err != nil {
		return err
	}
	defer os.Remove(f.Name())
	if err = f.Chmod(0600); err == nil {
		_, err = f.Write(b)
	}
	if err == nil {
		err = f.Sync()
	}
	closeErr := f.Close()
	if err == nil {
		err = closeErr
	}
	if err != nil {
		return err
	}
	return os.Rename(f.Name(), path)
}
func saveCredentials(dir string, c Credentials) error {
	b, err := json.Marshal(c)
	if err != nil {
		return err
	}
	return privateWrite(filepath.Join(dir, "credentials.json"), b)
}
func hostID(dir string) (string, error) {
	path := filepath.Join(dir, "host-id")
	b, err := os.ReadFile(path)
	if err == nil {
		id := strings.TrimSpace(string(b))
		if !strings.HasPrefix(id, "urn:uuid:") {
			return "", errors.New("invalid saved host ID")
		}
		return id, nil
	}
	if !os.IsNotExist(err) {
		return "", err
	}
	b = make([]byte, 16)
	if _, err = rand.Read(b); err != nil {
		return "", err
	}
	b[6] = (b[6] & 15) | 64
	b[8] = (b[8] & 63) | 128
	s := hex.EncodeToString(b)
	id := "urn:uuid:" + s[:8] + "-" + s[8:12] + "-" + s[12:16] + "-" + s[16:20] + "-" + s[20:]
	return id, privateWrite(path, []byte(id+"\n"))
}
func randomString() (string, error) {
	b := make([]byte, 32)
	_, err := rand.Read(b)
	return base64.RawURLEncoding.EncodeToString(b), err
}
func pkce(verifier string) string {
	h := sha256.Sum256([]byte(verifier))
	return base64.RawURLEncoding.EncodeToString(h[:])
}
func hasScope(scopes, want string) bool {
	for _, s := range strings.Fields(scopes) {
		if s == want {
			return true
		}
	}
	return false
}
func requirePlanScope(scope string) error {
	if !hasScope(scope, "chatgpt.tokens.use.direct") {
		return errors.New("ChatGPT plan permission was not granted; sign in again and enable plan usage")
	}
	return nil
}

type tokenResponse struct {
	AccessToken  string `json:"access_token"`
	RefreshToken string `json:"refresh_token"`
	IDToken      string `json:"id_token"`
	Scope        string `json:"scope"`
	TokenType    string `json:"token_type"`
	ExpiresIn    int64  `json:"expires_in"`
}

func exchangeToken(ctx context.Context, form url.Values) (tokenResponse, error) {
	var t tokenResponse
	req, err := http.NewRequestWithContext(ctx, "POST", authIssuer+"/api/accounts/oauth/token", strings.NewReader(form.Encode()))
	if err != nil {
		return t, err
	}
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	r, err := authHTTP.Do(req)
	if err != nil {
		return t, errors.New("token endpoint connection failed")
	}
	defer r.Body.Close()
	if r.StatusCode != 200 {
		return t, fmt.Errorf("token exchange failed (HTTP %d); sign in again if the session was revoked", r.StatusCode)
	}
	if err = json.NewDecoder(io.LimitReader(r.Body, 1<<20)).Decode(&t); err != nil {
		return t, errors.New("invalid token response")
	}
	if t.AccessToken == "" || t.ExpiresIn <= 0 || t.ExpiresIn > 31536000 || !strings.EqualFold(t.TokenType, "Bearer") {
		return t, errors.New("incomplete token response")
	}
	return t, nil
}
func readAuthJSON(ctx context.Context, uri string, out interface{}) error {
	req, err := http.NewRequestWithContext(ctx, "GET", uri, nil)
	if err != nil {
		return err
	}
	r, err := authHTTP.Do(req)
	if err != nil {
		return errors.New("identity verification connection failed")
	}
	defer r.Body.Close()
	if r.StatusCode != 200 {
		return fmt.Errorf("identity verification endpoint returned HTTP %d", r.StatusCode)
	}
	return json.NewDecoder(io.LimitReader(r.Body, 1<<20)).Decode(out)
}

type identityClaims struct {
	Issuer          string          `json:"iss"`
	Audience        json.RawMessage `json:"aud"`
	Subject         string          `json:"sub"`
	Email           string          `json:"email"`
	Nonce           string          `json:"nonce"`
	Expires         int64           `json:"exp"`
	NotBefore       int64           `json:"nbf"`
	AuthorizedParty string          `json:"azp"`
}

func validateClaims(c identityClaims, clientID, nonce string, now int64) error {
	var single string
	var audiences []string
	if json.Unmarshal(c.Audience, &single) == nil {
		audiences = []string{single}
	} else if json.Unmarshal(c.Audience, &audiences) != nil {
		return errors.New("invalid ID token audience")
	}
	found := false
	for _, a := range audiences {
		if a == clientID {
			found = true
		}
	}
	if c.Issuer != authIssuer || !found || c.Subject == "" || c.Expires <= now || c.NotBefore > now+30 {
		return errors.New("ID token identity claims failed validation")
	}
	if len(audiences) > 1 && c.AuthorizedParty != clientID {
		return errors.New("ID token authorized party does not match")
	}
	if c.AuthorizedParty != "" && c.AuthorizedParty != clientID {
		return errors.New("ID token authorized party does not match")
	}
	if nonce != "" && subtle.ConstantTimeCompare([]byte(c.Nonce), []byte(nonce)) != 1 {
		return errors.New("ID token nonce does not match")
	}
	return nil
}
func verifyIdentity(ctx context.Context, token, clientID, nonce string) (identityClaims, error) {
	var c identityClaims
	parts := strings.Split(token, ".")
	if len(parts) != 3 {
		return c, errors.New("invalid ID token")
	}
	var h struct {
		Alg string `json:"alg"`
		Kid string `json:"kid"`
	}
	b, err := base64.RawURLEncoding.DecodeString(parts[0])
	if err != nil {
		return c, errors.New("invalid ID token header")
	}
	if json.Unmarshal(b, &h) != nil || h.Alg != "RS256" || h.Kid == "" {
		return c, errors.New("unsupported ID token signature")
	}
	var discovery struct {
		Issuer string `json:"issuer"`
		JWKS   string `json:"jwks_uri"`
	}
	if err = readAuthJSON(ctx, authIssuer+"/.well-known/openid-configuration", &discovery); err != nil {
		return c, err
	}
	uri, err := url.Parse(discovery.JWKS)
	if err != nil || discovery.Issuer != authIssuer || uri.Scheme != "https" || uri.Host != "auth.openai.com" || uri.User != nil {
		return c, errors.New("invalid OpenAI identity discovery")
	}
	var jwks struct {
		Keys []struct {
			Kid string `json:"kid"`
			Kty string `json:"kty"`
			Use string `json:"use"`
			Alg string `json:"alg"`
			N   string `json:"n"`
			E   string `json:"e"`
		} `json:"keys"`
	}
	if err = readAuthJSON(ctx, discovery.JWKS, &jwks); err != nil {
		return c, err
	}
	var key *rsa.PublicKey
	for _, k := range jwks.Keys {
		if k.Kid != h.Kid || k.Kty != "RSA" || (k.Use != "" && k.Use != "sig") || (k.Alg != "" && k.Alg != "RS256") {
			continue
		}
		n, e1 := base64.RawURLEncoding.DecodeString(k.N)
		e, e2 := base64.RawURLEncoding.DecodeString(k.E)
		if e1 != nil || e2 != nil || len(e) > 4 {
			continue
		}
		exponent := 0
		for _, v := range e {
			exponent = exponent*256 + int(v)
		}
		if exponent < 3 {
			continue
		}
		key = &rsa.PublicKey{N: new(big.Int).SetBytes(n), E: exponent}
		if key.N.BitLen() < 2048 {
			key = nil
		}
		break
	}
	if key == nil {
		return c, errors.New("ID token signing key not found")
	}
	sig, err := base64.RawURLEncoding.DecodeString(parts[2])
	if err != nil {
		return c, errors.New("invalid ID token signature")
	}
	digest := sha256.Sum256([]byte(parts[0] + "." + parts[1]))
	if rsa.VerifyPKCS1v15(key, crypto.SHA256, digest[:], sig) != nil {
		return c, errors.New("ID token signature verification failed")
	}
	b, err = base64.RawURLEncoding.DecodeString(parts[1])
	if err != nil || json.Unmarshal(b, &c) != nil {
		return c, errors.New("invalid ID token claims")
	}
	return c, validateClaims(c, clientID, nonce, time.Now().Unix())
}

// Serialize rotating refresh tokens across both goroutines and processes.
func credentialsLock(dir string) (*os.File, error) {
	if err := os.MkdirAll(dir, 0700); err != nil {
		return nil, err
	}
	f, err := os.OpenFile(filepath.Join(dir, "credentials.lock"), os.O_CREATE|os.O_RDWR, 0600)
	if err != nil {
		return nil, err
	}
	if err = syscall.Flock(int(f.Fd()), syscall.LOCK_EX); err != nil {
		f.Close()
		return nil, err
	}
	return f, nil
}
func unlockCredentials(f *os.File) { syscall.Flock(int(f.Fd()), syscall.LOCK_UN); f.Close() }
func accessToken(dir string) (string, error) {
	lock, err := credentialsLock(dir)
	if err != nil {
		return "", err
	}
	defer unlockCredentials(lock)
	c, err := loadCredentials(dir)
	if err != nil {
		return "", errors.New("not signed in; run the login command")
	}
	if err = requirePlanScope(c.Scope); err != nil {
		return "", err
	}
	if c.AccessToken != "" && c.ExpiresAt > time.Now().Unix()+120 {
		return c.AccessToken, nil
	}
	if c.RefreshToken == "" || c.ClientID == "" || c.ClientID == "dynamic_agent_client" || c.Subject == "" {
		return "", errors.New("session cannot be refreshed; sign in again")
	}
	t, err := exchangeToken(context.Background(), url.Values{"grant_type": {"refresh_token"}, "client_id": {c.ClientID}, "refresh_token": {c.RefreshToken}, "resource": {authResource}})
	if err != nil {
		return "", err
	}
	if t.IDToken != "" {
		identity, verifyErr := verifyIdentity(context.Background(), t.IDToken, c.ClientID, "")
		if verifyErr != nil {
			return "", verifyErr
		}
		if identity.Subject != c.Subject {
			return "", errors.New("refreshed account identity changed")
		}
		c.IDToken = t.IDToken
		c.Email = identity.Email
	}
	if t.Scope != "" {
		c.Scope = t.Scope
	}
	c.AccessToken = t.AccessToken
	c.ExpiresAt = time.Now().Unix() + t.ExpiresIn
	if t.RefreshToken != "" {
		c.RefreshToken = t.RefreshToken
	}
	if err = saveCredentials(dir, c); err != nil {
		return "", err
	}
	if err = requirePlanScope(c.Scope); err != nil {
		return "", err
	}
	return c.AccessToken, nil
}

type authCallback struct {
	code, clientID string
	err            error
}

func parseCallback(q url.Values, state, clientID string) (authCallback, error) {
	if len(q["state"]) != 1 || subtle.ConstantTimeCompare([]byte(q.Get("state")), []byte(state)) != 1 {
		return authCallback{}, errors.New("authorization state does not match")
	}
	if q.Get("error") != "" {
		return authCallback{err: errors.New("sign-in was declined or failed")}, nil
	}
	if len(q["code"]) != 1 || q.Get("code") == "" {
		return authCallback{}, errors.New("authorization code missing")
	}
	issued := q.Get("client_id")
	if clientID == "dynamic_agent_client" {
		if len(q["client_id"]) != 1 || issued == "" || issued == "dynamic_agent_client" {
			return authCallback{}, errors.New("issued client ID missing")
		}
	} else {
		if issued != "" && issued != clientID {
			return authCallback{}, errors.New("issued client ID changed")
		}
		issued = clientID
	}
	return authCallback{code: q.Get("code"), clientID: issued}, nil
}
func login(dir string) error {
	lock, err := credentialsLock(dir)
	if err != nil {
		return err
	}
	defer unlockCredentials(lock)
	c, err := loadCredentials(dir)
	if err != nil && !os.IsNotExist(err) {
		return err
	}
	id, err := hostID(dir)
	if err != nil {
		return err
	}
	c.HostID = id
	clientID := c.ClientID
	if clientID == "" {
		clientID = "dynamic_agent_client"
	}
	state, err := randomString()
	if err != nil {
		return err
	}
	nonce, err := randomString()
	if err != nil {
		return err
	}
	verifier, err := randomString()
	if err != nil {
		return err
	}
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		return err
	}
	defer listener.Close()
	redirectURI := "http://" + listener.Addr().String() + "/auth/callback"
	q := url.Values{"client_id": {clientID}, "ext_agent_host_id": {id}, "response_type": {"code"}, "redirect_uri": {redirectURI}, "scope": {requestedScopes}, "resource": {authResource}, "state": {state}, "nonce": {nonce}, "code_challenge_method": {"S256"}, "code_challenge": {pkce(verifier)}}
	if clientID == "dynamic_agent_client" {
		q.Set("agent_name_hint", "Riddle")
	} else {
		if c.IDToken != "" {
			q.Set("id_token_hint", c.IDToken)
		}
		if c.Email != "" {
			q.Set("login_hint", c.Email)
		}
	}
	authorizeURL := authIssuer + "/api/accounts/authorize?" + q.Encode()
	results := make(chan authCallback, 1)
	mux := http.NewServeMux()
	startPath := "/auth/start/" + state
	// Print only the local link: returning authorization URLs contain a sensitive ID-token hint.
	mux.HandleFunc(startPath, func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Cache-Control", "no-store")
		http.Redirect(w, r, authorizeURL, http.StatusFound)
	})
	mux.HandleFunc("/auth/callback", func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Cache-Control", "no-store")
		w.Header().Set("Content-Type", "text/plain; charset=utf-8")
		if r.Method != "GET" {
			http.Error(w, "GET required", 405)
			return
		}
		cb, e := parseCallback(r.URL.Query(), state, clientID)
		if e != nil {
			http.Error(w, "Invalid sign-in callback.", 400)
			return
		}
		select {
		case results <- cb:
			fmt.Fprintln(w, "Sign-in received. You may close this tab and return to Riddle.")
		default:
			http.Error(w, "Sign-in callback already received.", 409)
		}
	})
	server := &http.Server{Handler: mux, ReadHeaderTimeout: 5 * time.Second}
	defer server.Close()
	serveErrors := make(chan error, 1)
	go func() { serveErrors <- server.Serve(listener) }()
	fmt.Println("Continue with ChatGPT: http://" + listener.Addr().String() + startPath)
	interruptCtx, cancelInterrupt := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer cancelInterrupt()
	ctx, cancel := context.WithTimeout(interruptCtx, 10*time.Minute)
	defer cancel()
	var cb authCallback
	select {
	case cb = <-results:
	case err = <-serveErrors:
		return err
	case <-ctx.Done():
		return errors.New("sign-in cancelled or timed out")
	}
	if cb.err != nil {
		return cb.err
	}
	c.ClientID = cb.clientID
	if err = saveCredentials(dir, c); err != nil {
		return err
	} // Retain registration even if the code expires.
	t, err := exchangeToken(ctx, url.Values{"grant_type": {"authorization_code"}, "client_id": {cb.clientID}, "code": {cb.code}, "code_verifier": {verifier}, "redirect_uri": {redirectURI}, "resource": {authResource}})
	if err != nil {
		return err
	}
	if t.IDToken == "" {
		return errors.New("ID token missing from sign-in response")
	}
	identity, err := verifyIdentity(ctx, t.IDToken, cb.clientID, nonce)
	if err != nil {
		return err
	}
	if c.Subject != "" && identity.Subject != c.Subject {
		return errors.New("sign-in account differs from saved account")
	}
	c.Subject = identity.Subject
	c.Email = identity.Email
	c.IDToken = t.IDToken
	c.AccessToken = t.AccessToken
	c.RefreshToken = t.RefreshToken
	c.Scope = t.Scope
	c.ExpiresAt = time.Now().Unix() + t.ExpiresIn
	if err = saveCredentials(dir, c); err != nil {
		return err
	}
	if err = requirePlanScope(c.Scope); err != nil {
		return err
	}
	fmt.Println("ChatGPT connected. Manage app limits at https://chatgpt.com/settings/usage")
	return nil
}
