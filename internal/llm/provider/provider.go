// Package provider is the chat-API vocabulary: which provider, which model, which URL.
package provider

import (
	"encoding/json"
	"fmt"
	"strings"
)

// The chat APIs this app can talk to. Everything except Gemini is OpenAI-compatible, so
// the difference is the base URL, the model name and — for Gemini — the request shape.
type Provider string

const (
	Deepseek Provider = "deepseek"
	Openai   Provider = "openai"
	Gemini   Provider = "gemini"
	// Any OpenAI-compatible endpoint: OpenRouter, Groq, Ollama, a proxy.
	Custom Provider = "custom"
)

func ParseProvider(s string) (Provider, error) {
	switch strings.ToLower(strings.TrimSpace(s)) {
	case "", "deepseek":
		return Deepseek, nil
	case "openai":
		return Openai, nil
	case "gemini", "google":
		return Gemini, nil
	case "custom", "openai-compatible":
		return Custom, nil
	}
	return "", fmt.Errorf("unknown model provider %q", s)
}

func (p Provider) DefaultModel() string {
	switch p {
	case Deepseek:
		return "deepseek-chat"
	case Openai:
		return "gpt-4o-mini"
	case Gemini:
		return "gemini-2.0-flash"
	}
	return ""
}

func (p Provider) DefaultBaseURL() string {
	switch p {
	case Deepseek:
		return "https://api.deepseek.com"
	case Openai:
		return "https://api.openai.com/v1"
	case Gemini:
		return "https://generativelanguage.googleapis.com/v1beta"
	}
	return ""
}

// Only DeepSeek offers server-side page fetch, which the escalation step uses to read an
// article the model asked for.
func (p Provider) SupportsArticleFetch() bool {
	return p == Deepseek
}

// The resolved settings for one call.
type ModelConfig struct {
	Provider Provider
	Model    string
	BaseURL  string
	APIKey   string
}

func (c ModelConfig) URL(path string) string {
	return strings.TrimRight(c.BaseURL, "/") + "/" + strings.TrimLeft(path, "/")
}

// A Gemini key travels in the query string, not in an Authorization header.
func (c ModelConfig) ChatURL() string {
	if c.Provider == Gemini {
		return c.URL(fmt.Sprintf("models/%s:generateContent", c.Model)) + "?key=" + c.APIKey
	}
	return c.URL("chat/completions")
}

func (c ModelConfig) RequestBody(prompt string) json.RawMessage {
	if c.Provider == Gemini {
		b, _ := json.Marshal(map[string]any{
			"contents": []map[string]any{
				{"role": "user", "parts": []map[string]any{{"text": prompt}}},
			},
			"generationConfig": map[string]any{
				"responseMimeType": "application/json",
				"temperature":      0.2,
			},
		})
		return b
	}
	b, _ := json.Marshal(map[string]any{
		"model":    c.Model,
		"messages": []map[string]any{{"role": "user", "content": prompt}},
		"response_format": map[string]any{
			"type": "json_object",
		},
		"temperature": 0.2,
	})
	return b
}

func pointer(m map[string]any, path ...string) any {
	var cur any = m
	for _, p := range path {
		switch node := cur.(type) {
		case map[string]any:
			cur = node[p]
		case []any:
			var idx int
			if _, err := fmt.Sscanf(p, "%d", &idx); err != nil || idx < 0 || idx >= len(node) {
				return nil
			}
			cur = node[idx]
		default:
			return nil
		}
	}
	return cur
}

func asStr(v any) (string, bool) {
	s, ok := v.(string)
	return s, ok
}

func asInt(v any) int64 {
	if f, ok := v.(float64); ok {
		return int64(f)
	}
	return 0
}

// Pull the assistant text out of whichever envelope came back.
func (c ModelConfig) ExtractContent(body map[string]any) (string, bool) {
	if c.Provider == Gemini {
		if s, ok := asStr(pointer(body, "candidates", "0", "content", "parts", "0", "text")); ok {
			return s, true
		}
		return "", false
	}
	if s, ok := asStr(pointer(body, "choices", "0", "message", "content")); ok {
		return s, true
	}
	return "", false
}

func (c ModelConfig) Usage(body map[string]any) (int64, int64) {
	if c.Provider == Gemini {
		return asInt(pointer(body, "usageMetadata", "promptTokenCount")),
			asInt(pointer(body, "usageMetadata", "candidatesTokenCount"))
	}
	return asInt(pointer(body, "usage", "prompt_tokens")),
		asInt(pointer(body, "usage", "completion_tokens"))
}
