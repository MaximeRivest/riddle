package main

import (
	"bytes"
	"encoding/base64"
	"encoding/json"
	"image"
	"image/png"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

const testSecret = "a-local-secret-that-is-at-least-32-bytes"

func pagePNG(t *testing.T) string {
	t.Helper()
	var data bytes.Buffer
	if err := png.Encode(&data, image.NewRGBA(image.Rect(0, 0, 2, 3))); err != nil {
		t.Fatal(err)
	}
	return "data:image/png;base64," + base64.StdEncoding.EncodeToString(data.Bytes())
}

func realisticRequest(t *testing.T) []byte {
	t.Helper()
	value := map[string]interface{}{"model": "riddle-default", "stream": true, "messages": []interface{}{
		map[string]interface{}{"role": "system", "content": "Reply briefly and finish with a transcription."},
		map[string]interface{}{"role": "user", "content": "Previous question"},
		map[string]interface{}{"role": "assistant", "content": "Previous answer"},
		map[string]interface{}{"role": "user", "content": []interface{}{map[string]string{"type": "text", "text": "Read the handwritten page."}, map[string]interface{}{"type": "image_url", "image_url": map[string]string{"url": pagePNG(t)}}}},
	}}
	data, err := json.Marshal(value)
	if err != nil {
		t.Fatal(err)
	}
	return data
}

func TestConvertDiaryMessages(t *testing.T) {
	var request diaryRequest
	if err := json.Unmarshal(realisticRequest(t), &request); err != nil {
		t.Fatal(err)
	}
	result, err := convertMessages(request.Messages)
	if err != nil {
		t.Fatal(err)
	}
	if result[0].Role != "developer" || result[0].Content[0].Type != "input_text" {
		t.Fatal("system prompt lost")
	}
	if result[2].Role != "assistant" || result[2].Content[0].Type != "output_text" {
		t.Fatal("assistant history incorrectly converted")
	}
	if result[3].Content[1].Type != "input_image" || !strings.HasPrefix(result[3].Content[1].ImageURL, "data:image/png;base64,") {
		t.Fatal("page image lost")
	}
}

func TestRejectInvalidImagesAndRoles(t *testing.T) {
	for _, url := range []string{"https://example.com/page.png", "data:image/jpeg;base64,YQ==", "data:image/png;base64,YQ==", pagePNG(t) + "invalid"} {
		if validatePNG(url) == nil {
			t.Errorf("accepted invalid image %q", url[:min(len(url), 40)])
		}
	}
	if _, err := convertMessages([]diaryMessage{{Role: "tool", Content: json.RawMessage(`"x"`)}}); err == nil {
		t.Fatal("accepted tool role")
	}
	if _, err := convertMessages(nil); err == nil {
		t.Fatal("accepted missing messages")
	}
	var request diaryRequest
	_ = json.Unmarshal(realisticRequest(t), &request)
	request.Messages[3].Role = "assistant"
	if _, err := convertMessages(request.Messages); err == nil {
		t.Fatal("accepted assistant image")
	}
}

func TestRelayBearerAndHealth(t *testing.T) {
	calls := 0
	handler := newRelayHandler(t.TempDir(), testSecret, "model", nil, "http://invalid", func() (string, error) { calls++; return "oauth-secret", nil })
	for _, authorization := range []string{"", "Bearer wrong", "Basic " + testSecret} {
		req := httptest.NewRequest("POST", "/v1/chat/completions", bytes.NewReader(realisticRequest(t)))
		req.Header.Set("Authorization", authorization)
		result := httptest.NewRecorder()
		handler.ServeHTTP(result, req)
		if result.Code != 401 {
			t.Fatalf("expected401 got%d", result.Code)
		}
	}
	if calls != 0 {
		t.Fatal("unauthorized request acquired OAuth token")
	}
	result := httptest.NewRecorder()
	handler.ServeHTTP(result, httptest.NewRequest("GET", "/health", nil))
	if result.Code != 200 || strings.Contains(result.Body.String(), "secret") {
		t.Fatal("bad health response")
	}
}

func TestRelayResponsesEndToEnd(t *testing.T) {
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
		if req.Header.Get("Authorization") != "Bearer oauth-secret" {
			t.Error("OAuth token missing upstream")
		}
		if req.URL.Path == "/models" {
			_, _ = io.WriteString(w, `{"models":[{"slug":"hidden","visibility":"hide"},{"slug":"chosen-model","visibility":"list"}]}`)
			return
		}
		if req.URL.Path != "/responses" {
			t.Errorf("unexpected path %s", req.URL.Path)
		}
		var body map[string]json.RawMessage
		if err := json.NewDecoder(req.Body).Decode(&body); err != nil {
			t.Error(err)
		}
		if string(body["store"]) != "false" || string(body["stream"]) != "true" || string(body["model"]) != `"chosen-model"` {
			t.Errorf("bad response request: %s", body)
		}
		if string(body["max_output_tokens"]) != "2000" {
			t.Error("default output cap missing upstream")
		}
		if _, exists := body["reasoning"]; exists {
			t.Error("reasoning added without a diary request")
		}
		if _, ok := body["tools"]; ok {
			t.Error("tools must be absent")
		}
		var input []responseMessage
		_ = json.Unmarshal(body["input"], &input)
		if len(input) != 4 || input[3].Content[1].Type != "input_image" {
			t.Error("request context missing")
		}
		w.Header().Set("Content-Type", "text/event-stream")
		_, _ = io.WriteString(w, "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello.\"}\n\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\" ⁂ test\"}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n\n")
	}))
	defer upstream.Close()
	handler := newRelayHandler(t.TempDir(), testSecret, "", upstream.Client(), upstream.URL, func() (string, error) { return "oauth-secret", nil })
	request := httptest.NewRequest("POST", "/v1/chat/completions", bytes.NewReader(realisticRequest(t)))
	request.Header.Set("Authorization", "Bearer "+testSecret)
	result := httptest.NewRecorder()
	handler.ServeHTTP(result, request)
	if result.Code != 200 {
		t.Fatalf("HTTP %d %s", result.Code, result.Body.String())
	}
	for _, expected := range []string{`"delta":{"content":"Hello."}`, "⁂ test", "data: [DONE]"} {
		if !strings.Contains(result.Body.String(), expected) {
			t.Errorf("missing %q in %s", expected, result.Body.String())
		}
	}
	if strings.Contains(result.Body.String(), "oauth-secret") {
		t.Fatal("OAuth token leaked downstream")
	}
}

func TestUpstreamFailuresNeverComplete(t *testing.T) {
	for name, stream := range map[string]string{
		"truncated":   "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Partial\"}\n\n",
		"incomplete":  "data: {\"type\":\"response.incomplete\",\"response\":{\"status\":\"incomplete\"}}\n\n",
		"limit":       "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"subscription_sharing_usage_limit_exceeded\"}}}\n\n",
		"malformed":   "data: invalid JSON\n\n",
		"wrongStatus": "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"failed\"}}\n\n",
	} {
		t.Run(name, func(t *testing.T) {
			upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) { _, _ = io.WriteString(w, stream) }))
			defer upstream.Close()
			handler := newRelayHandler(t.TempDir(), testSecret, "model", upstream.Client(), upstream.URL, func() (string, error) { return "oauth", nil })
			req := httptest.NewRequest("POST", "/v1/chat/completions", bytes.NewReader(realisticRequest(t)))
			req.Header.Set("Authorization", "Bearer "+testSecret)
			result := httptest.NewRecorder()
			handler.ServeHTTP(result, req)
			if strings.Contains(result.Body.String(), "[DONE]") || !strings.Contains(result.Body.String(), "connection error") {
				t.Fatal("failure reported as success", result.Body.String())
			}
		})
	}
}

func TestLoopbackOnly(t *testing.T) {
	if err := serveRelay(t.TempDir(), "0.0.0.0:8787", testSecret, "model"); err == nil {
		t.Fatal("accepted non-loopback listener")
	}
	if err := serveRelay(t.TempDir(), "127.0.0.1:8787", "short", "model"); err == nil {
		t.Fatal("accepted short secret")
	}
}

func TestResponsesTokenAndReasoningOptions(t *testing.T) {
	for _, test := range []struct {
		name, body string
		limit      int
		effort     string
		invalid    bool
	}{
		{name: "defaults", body: `{}`, limit: 2000},
		{name: "legacyCap", body: `{"max_tokens":3000}`, limit: 3000},
		{name: "completionCap", body: `{"max_completion_tokens":4000}`, limit: 4000},
		{name: "completionWins", body: `{"max_tokens":1000,"max_completion_tokens":4500}`, limit: 4500},
		{name: "clamped", body: `{"max_tokens":99999}`, limit: 16384},
		{name: "low", body: `{"reasoning_effort":"low"}`, limit: 2000, effort: "low"},
		{name: "minimal", body: `{"reasoning_effort":"minimal"}`, limit: 2000, effort: "minimal"},
		{name: "medium", body: `{"reasoning_effort":"medium"}`, limit: 2000, effort: "medium"},
		{name: "high", body: `{"reasoning_effort":"high"}`, limit: 2000, effort: "high"},
		{name: "zero", body: `{"max_tokens":0}`, invalid: true},
		{name: "negative", body: `{"max_completion_tokens":-1}`, invalid: true},
		{name: "invalidUnusedCap", body: `{"max_tokens":0,"max_completion_tokens":2000}`, invalid: true},
		{name: "invalidEffort", body: `{"reasoning_effort":"invented"}`, invalid: true},
	} {
		t.Run(test.name, func(t *testing.T) {
			var diary diaryRequest
			if err := json.Unmarshal([]byte(test.body), &diary); err != nil {
				t.Fatal(err)
			}
			payload, err := responsesPayload(diary, nil)
			if test.invalid {
				if err == nil {
					t.Fatal("accepted invalid generation options")
				}
				return
			}
			if err != nil {
				t.Fatal(err)
			}
			if payload["max_output_tokens"] != test.limit {
				t.Fatalf("token limit %v, want %d", payload["max_output_tokens"], test.limit)
			}
			if test.effort == "" {
				if _, exists := payload["reasoning"]; exists {
					t.Fatal("reasoning added when absent")
				}
			} else {
				options, ok := payload["reasoning"].(map[string]string)
				if !ok || options["effort"] != test.effort {
					t.Fatal("reasoning effort was lost")
				}
			}
		})
	}
}
