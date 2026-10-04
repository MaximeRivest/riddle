package main

// The local diary endpoint translates Chat Completions into the public
// Responses API. Its bearer secret is unrelated to the ChatGPT OAuth token.
import (
	"bufio"
	"bytes"
	"context"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/base64"
	"encoding/binary"
	"encoding/json"
	"errors"
	"fmt"
	"hash/crc32"
	"image/png"
	"io"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"time"
)

const maxRequestBytes = 16 << 20
const maxImageBytes = 8 << 20

type diaryMessage struct {
	Role    string          `json:"role"`
	Content json.RawMessage `json:"content"`
}
type diaryRequest struct {
	Messages            []diaryMessage `json:"messages"`
	Model               string         `json:"model"`
	MaxTokens           *int           `json:"max_tokens"`
	MaxCompletionTokens *int           `json:"max_completion_tokens"`
	ReasoningEffort     string         `json:"reasoning_effort,omitempty"`
}

func responsesPayload(diary diaryRequest, input []responseMessage) (map[string]interface{}, error) {
	limit := 2000
	for _, requested := range []*int{diary.MaxTokens, diary.MaxCompletionTokens} {
		if requested != nil {
			if *requested <= 0 {
				return nil, errors.New("token limit must be positive")
			}
			limit = *requested
		}
	}
	if limit > 16384 {
		limit = 16384
	}
	payload := map[string]interface{}{"input": input, "store": false, "stream": true, "max_output_tokens": limit}
	if diary.ReasoningEffort != "" {
		switch diary.ReasoningEffort {
		case "minimal", "low", "medium", "high":
			payload["reasoning"] = map[string]string{"effort": diary.ReasoningEffort}
		default:
			return nil, errors.New("unsupported reasoning effort")
		}
	}
	return payload, nil
}

type responseContent struct {
	Type     string `json:"type"`
	Text     string `json:"text,omitempty"`
	ImageURL string `json:"image_url,omitempty"`
	Detail   string `json:"detail,omitempty"`
}
type responseMessage struct {
	Role    string            `json:"role"`
	Content []responseContent `json:"content"`
}

func validatePNG(url string) error {
	const prefix = "data:image/png;base64,"
	if !strings.HasPrefix(url, prefix) {
		return errors.New("only inline base64 PNG images are supported")
	}
	encoded := strings.TrimPrefix(url, prefix)
	if len(encoded) > base64.StdEncoding.EncodedLen(maxImageBytes) {
		return errors.New("PNG exceeds 8 MiB")
	}
	data, err := base64.StdEncoding.Strict().DecodeString(encoded)
	if err != nil || len(data) > maxImageBytes {
		return errors.New("invalid PNG base64")
	}
	cfg, err := png.DecodeConfig(bytes.NewReader(data))
	if err != nil || cfg.Width < 1 || cfg.Height < 1 || cfg.Width > 10000 || cfg.Height > 10000 || int64(cfg.Width)*int64(cfg.Height) > 30000000 {
		return errors.New("invalid PNG or excessive dimensions")
	}
	// Check all chunk boundaries and checksums without allocating decoded pixels.
	pos, idat := 8, false
	for pos+12 <= len(data) {
		n := int64(binary.BigEndian.Uint32(data[pos : pos+4]))
		if n > int64(len(data)-pos-12) {
			return errors.New("truncated PNG chunk")
		}
		end := pos + 8 + int(n)
		if crc32.ChecksumIEEE(data[pos+4:end]) != binary.BigEndian.Uint32(data[end:end+4]) {
			return errors.New("invalid PNG checksum")
		}
		kind := string(data[pos+4 : pos+8])
		if kind == "IDAT" {
			idat = true
		}
		if kind == "IEND" {
			if n != 0 || end+4 != len(data) || !idat {
				return errors.New("invalid PNG end")
			}
			return nil
		}
		pos = end + 4
	}
	return errors.New("PNG is missing its end chunk")
}

func convertMessages(messages []diaryMessage) ([]responseMessage, error) {
	if len(messages) == 0 || len(messages) > 100 {
		return nil, errors.New("expected 1 to 100 messages")
	}
	result := make([]responseMessage, 0, len(messages))
	textSize, imageCount := 0, 0
	for _, message := range messages {
		if bytes.Equal(bytes.TrimSpace(message.Content), []byte("null")) {
			return nil, errors.New("message content cannot be null")
		}
		role := message.Role
		if role == "system" {
			role = "developer"
		}
		if role != "developer" && role != "user" && role != "assistant" {
			return nil, errors.New("unsupported message role")
		}
		textType := "input_text"
		if role == "assistant" {
			textType = "output_text"
		}
		var text string
		var parts []struct {
			Type     string `json:"type"`
			Text     string `json:"text"`
			ImageURL struct {
				URL string `json:"url"`
			} `json:"image_url"`
		}
		content := make([]responseContent, 0)
		if err := json.Unmarshal(message.Content, &text); err == nil {
			if text == "" {
				return nil, errors.New("message text cannot be empty")
			}
			textSize += len(text)
			content = append(content, responseContent{Type: textType, Text: text})
		} else {
			if err := json.Unmarshal(message.Content, &parts); err != nil || len(parts) == 0 {
				return nil, errors.New("invalid message content")
			}
			for _, part := range parts {
				switch part.Type {
				case "text":
					if part.Text == "" {
						return nil, errors.New("message text cannot be empty")
					}
					textSize += len(part.Text)
					content = append(content, responseContent{Type: textType, Text: part.Text})
				case "image_url":
					if role != "user" {
						return nil, errors.New("images must belong to user messages")
					}
					imageCount++
					if imageCount > 4 {
						return nil, errors.New("at most four page images are allowed")
					}
					if err := validatePNG(part.ImageURL.URL); err != nil {
						return nil, err
					}
					content = append(content, responseContent{Type: "input_image", ImageURL: part.ImageURL.URL, Detail: "high"})
				default:
					return nil, errors.New("unsupported content type")
				}
			}
		}
		if textSize > 1<<20 {
			return nil, errors.New("text exceeds 1 MiB")
		}
		result = append(result, responseMessage{Role: role, Content: content})
	}
	return result, nil
}

type relay struct {
	stateDir, secret, model, baseURL string
	client                           *http.Client
	tokenSource                      func() (string, error)
	busy                             chan struct{}
}

func newRelayHandler(stateDir, localToken, model string, client *http.Client, baseURL string, tokenSource func() (string, error)) http.Handler {
	if client == nil {
		client = &http.Client{Timeout: 180 * time.Second, CheckRedirect: func(req *http.Request, via []*http.Request) error { return http.ErrUseLastResponse }}
	}
	if tokenSource == nil {
		tokenSource = func() (string, error) { return accessToken(stateDir) }
	}
	return &relay{stateDir: stateDir, secret: localToken, model: model, baseURL: strings.TrimRight(baseURL, "/"), client: client, tokenSource: tokenSource, busy: make(chan struct{}, 1)}
}

func serveRelay(stateDir, listen, localToken, model string) error {
	host, _, err := net.SplitHostPort(listen)
	if err != nil || host != "127.0.0.1" {
		return errors.New("relay must listen on 127.0.0.1")
	}
	if len(localToken) < 32 {
		return errors.New("relay secret must contain at least 32 characters")
	}
	server := &http.Server{Addr: listen, Handler: newRelayHandler(stateDir, localToken, model, nil, "https://api.openai.com/v1", nil), ReadHeaderTimeout: 5 * time.Second, ReadTimeout: 20 * time.Second, IdleTimeout: 30 * time.Second, MaxHeaderBytes: 16 << 10}
	return server.ListenAndServe()
}

func (r *relay) authorized(header string) bool {
	got := sha256.Sum256([]byte(header))
	expected := sha256.Sum256([]byte("Bearer " + r.secret))
	return len(r.secret) >= 32 && subtle.ConstantTimeCompare(got[:], expected[:]) == 1
}

func jsonError(w http.ResponseWriter, status int, message string) {
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Cache-Control", "no-store")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(map[string]interface{}{"error": map[string]string{"message": message}})
}

func (r *relay) chooseModel(ctx context.Context, token string) (string, error) {
	if r.model != "" {
		return r.model, nil
	}
	if data, err := os.ReadFile(filepath.Join(r.stateDir, "model.txt")); err == nil && strings.TrimSpace(string(data)) != "" {
		return strings.TrimSpace(string(data)), nil
	}
	req, err := http.NewRequestWithContext(ctx, "GET", r.baseURL+"/models", nil)
	if err != nil {
		return "", err
	}
	req.Header.Set("Authorization", "Bearer "+token)
	resp, err := r.client.Do(req)
	if err != nil {
		return "", errors.New("model catalog connection failed")
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return "", fmt.Errorf("model catalog returned HTTP %d", resp.StatusCode)
	}
	var catalog struct {
		Models []struct {
			Slug       string `json:"slug"`
			Visibility string `json:"visibility"`
		} `json:"models"`
	}
	if err := json.NewDecoder(io.LimitReader(resp.Body, 2<<20)).Decode(&catalog); err != nil {
		return "", errors.New("invalid model catalog")
	}
	for _, model := range catalog.Models {
		if model.Visibility == "list" && model.Slug != "" {
			return model.Slug, nil
		}
	}
	return "", errors.New("no model available for this ChatGPT account")
}

func (r *relay) ServeHTTP(w http.ResponseWriter, req *http.Request) {
	if req.Method == "GET" && req.URL.Path == "/health" {
		w.Header().Set("Content-Type", "application/json")
		w.Header().Set("Cache-Control", "no-store")
		_, _ = io.WriteString(w, `{"status":"ok","backend":"chatgpt-plan"}`)
		return
	}
	if req.URL.Path != "/v1/chat/completions" {
		jsonError(w, 404, "unknown endpoint")
		return
	}
	if req.Method != "POST" {
		jsonError(w, 405, "POST required")
		return
	}
	if !r.authorized(req.Header.Get("Authorization")) {
		jsonError(w, 401, "invalid local relay secret")
		return
	}
	select {
	case r.busy <- struct{}{}:
		defer func() { <-r.busy }()
	default:
		jsonError(w, 429, "diary is already answering a request")
		return
	}
	req.Body = http.MaxBytesReader(w, req.Body, maxRequestBytes)
	var diary diaryRequest
	decoder := json.NewDecoder(req.Body)
	if err := decoder.Decode(&diary); err != nil {
		jsonError(w, 400, "invalid or oversized diary request")
		return
	}
	var extra interface{}
	if err := decoder.Decode(&extra); err != io.EOF {
		jsonError(w, 400, "request must contain one JSON object")
		return
	}
	input, err := convertMessages(diary.Messages)
	if err != nil {
		jsonError(w, 400, err.Error())
		return
	}
	payload, err := responsesPayload(diary, input)
	if err != nil {
		jsonError(w, 400, err.Error())
		return
	}
	ctx, cancel := context.WithTimeout(req.Context(), 180*time.Second)
	defer cancel()
	token, err := r.tokenSource()
	if err != nil || token == "" {
		jsonError(w, 503, "ChatGPT sign-in or token renewal required")
		return
	}
	model, err := r.chooseModel(ctx, token)
	if err != nil {
		jsonError(w, 502, err.Error())
		return
	}
	// The configured account model wins over the generic model sent by Riddle.
	payload["model"] = model
	body, err := json.Marshal(payload)
	if err != nil {
		jsonError(w, 400, "invalid diary input")
		return
	}
	upstream, err := http.NewRequestWithContext(ctx, "POST", r.baseURL+"/responses", bytes.NewReader(body))
	if err != nil {
		jsonError(w, 502, "invalid upstream configuration")
		return
	}
	upstream.Header.Set("Authorization", "Bearer "+token)
	upstream.Header.Set("Content-Type", "application/json")
	upstream.Header.Set("Accept", "text/event-stream")
	resp, err := r.client.Do(upstream)
	if err != nil {
		jsonError(w, 502, "ChatGPT connection failed")
		return
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		jsonError(w, 502, fmt.Sprintf("ChatGPT returned HTTP %d; check sign-in and plan limits", resp.StatusCode))
		return
	}
	flusher, ok := w.(http.Flusher)
	if !ok {
		jsonError(w, 500, "streaming unavailable")
		return
	}
	w.Header().Set("Content-Type", "text/event-stream")
	w.Header().Set("Cache-Control", "no-store")
	w.Header().Set("X-Accel-Buffering", "no")
	w.WriteHeader(http.StatusOK)
	_, _ = io.WriteString(w, ": connected\n\n")
	flusher.Flush()
	streamResponse(ctx, w, flusher, resp.Body, model)
}

type streamEvent struct {
	data string
	err  error
}

func readSSE(ctx context.Context, body io.Reader, events chan<- streamEvent) {
	defer close(events)
	scanner := bufio.NewScanner(body)
	scanner.Buffer(make([]byte, 4096), 2<<20)
	var data strings.Builder
	send := func(event streamEvent) bool {
		select {
		case events <- event:
			return true
		case <-ctx.Done():
			return false
		}
	}
	for scanner.Scan() {
		line := scanner.Text()
		if line == "" {
			if data.Len() > 0 {
				if !send(streamEvent{data: data.String()}) {
					return
				}
				data.Reset()
			}
		} else if strings.HasPrefix(line, "data:") {
			if data.Len() > 0 {
				data.WriteByte('\n')
			}
			data.WriteString(strings.TrimPrefix(strings.TrimPrefix(line, "data:"), " "))
			if data.Len() > 2<<20 {
				send(streamEvent{err: errors.New("oversized upstream event")})
				return
			}
		}
	}
	if data.Len() > 0 && !send(streamEvent{data: data.String()}) {
		return
	}
	if err := scanner.Err(); err != nil {
		send(streamEvent{err: err})
	}
}

func streamResponse(ctx context.Context, w io.Writer, flusher http.Flusher, body io.Reader, model string) {
	readCtx, cancel := context.WithCancel(ctx)
	defer cancel()
	events := make(chan streamEvent)
	go readSSE(readCtx, body, events)
	ticker := time.NewTicker(15 * time.Second)
	defer ticker.Stop()
	chunk := func(text string) bool {
		data, _ := json.Marshal(map[string]interface{}{"object": "chat.completion.chunk", "model": model, "choices": []interface{}{map[string]interface{}{"index": 0, "delta": map[string]string{"content": text}}}})
		_, err := fmt.Fprintf(w, "data: %s\n\n", data)
		flusher.Flush()
		return err == nil
	}
	fail := func(message string) { chunk("\n[ChatGPT connection error: " + message + "]") }
	for {
		select {
		case <-ctx.Done():
			fail("request timed out or was canceled; response is unfinished")
			return
		case <-ticker.C:
			if _, err := io.WriteString(w, ": waiting\n\n"); err != nil {
				return
			}
			flusher.Flush()
		case event, open := <-events:
			if !open || event.err != nil {
				fail("stream ended before completion; retry this page")
				return
			}
			var value struct {
				Type     string `json:"type"`
				Delta    string `json:"delta"`
				Response struct {
					Status string `json:"status"`
					Error  *struct {
						Code string `json:"code"`
					} `json:"error"`
				} `json:"response"`
			}
			if err := json.Unmarshal([]byte(event.data), &value); err != nil {
				fail("invalid upstream stream")
				return
			}
			switch value.Type {
			case "response.output_text.delta":
				if !chunk(value.Delta) {
					return
				}
			case "response.completed":
				if value.Response.Status != "completed" {
					fail("response did not complete")
					return
				}
				_, _ = io.WriteString(w, "data: [DONE]\n\n")
				flusher.Flush()
				return
			case "response.failed", "response.incomplete", "error":
				message := "response failed or is incomplete; check ChatGPT plan limits and retry"
				if value.Response.Error != nil && strings.HasPrefix(value.Response.Error.Code, "subscription_sharing_") {
					message = "ChatGPT plan usage is unavailable or its limit was reached; review ChatGPT usage settings"
				}
				fail(message)
				return
			}
		}
	}
}
