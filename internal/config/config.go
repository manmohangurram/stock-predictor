package config

import (
	"encoding/base64"
	"encoding/hex"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

// Deployment facts only: where to listen, where the database lives, and the address the
// app is reachable at. Everything a user edits in the UI lives in the settings table and
// is never read from the environment.
type Config struct {
	DBPath    string
	Bind      string
	PublicURL string
	SecretKey []byte
}

// The variable a developer's .env may carry a model key in, per provider. Read only when
// the database has no key for that provider.
func KeyEnvName(provider string) string {
	switch provider {
	case "deepseek":
		return "DEEPSEEK_API_KEY"
	case "openai":
		return "OPENAI_API_KEY"
	case "gemini":
		return "GEMINI_API_KEY"
	case "custom":
		return "CUSTOM_API_KEY"
	}
	return ""
}

// Variables that used to configure the app and are now settings. Boot warns when one is
// still set, because a silently ignored POLL_SECS=600 is an evening lost to wondering.
var Retired = []string{
	"POLL_SECS",
	"OFF_HOURS_SECS",
	"LLM_DAILY_CAP",
	"LLM_MIN_PRICE_MOVE",
	"LLM_MAX_SILENCE_SECS",
	"PROMPT_WINDOW_HOURS",
	"DEEPSEEK_BASE_URL",
}

func LoadDotenv() []string {
	raw, err := os.ReadFile(".env")
	if err != nil {
		return nil
	}
	loaded := []string{}
	for _, kv := range ParseDotenv(string(raw)) {
		if _, ok := os.LookupEnv(kv[0]); !ok {
			os.Setenv(kv[0], kv[1])
			loaded = append(loaded, kv[0])
		}
	}
	return loaded
}

// KEY=value, KEY="a b", # comment, blank lines. Quotes are stripped, export ignored.
func ParseDotenv(raw string) [][2]string {
	out := [][2]string{}
	for _, line := range strings.Split(raw, "\n") {
		line = strings.TrimSpace(line)
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		line = strings.TrimPrefix(line, "export ")
		key, value, ok := strings.Cut(line, "=")
		if !ok {
			continue
		}
		key = strings.TrimSpace(key)
		if key == "" || !validKey(key) {
			continue
		}
		value = strings.TrimSpace(value)
		if len(value) >= 2 &&
			((value[0] == '"' && value[len(value)-1] == '"') ||
				(value[0] == '\'' && value[len(value)-1] == '\'')) {
			value = value[1 : len(value)-1]
		} else if i := strings.Index(value, " #"); i >= 0 {
			value = strings.TrimSpace(value[:i])
		}
		out = append(out, [2]string{key, value})
	}
	return out
}

func validKey(key string) bool {
	for _, r := range key {
		if !(r >= 'a' && r <= 'z' || r >= 'A' && r <= 'Z' || r >= '0' && r <= '9' || r == '_') {
			return false
		}
	}
	return true
}

// 32 bytes as 64 hex characters, or base64 (44 with padding, 43 without).
func parseKey(raw string) ([]byte, error) {
	if len(raw) == 64 {
		if b, err := hex.DecodeString(raw); err == nil && len(b) == 32 {
			return b, nil
		}
	}
	if b, err := base64.StdEncoding.DecodeString(raw); err == nil && len(b) == 32 {
		return b, nil
	}
	return nil, fmt.Errorf("SECRET_KEY must be 32 bytes: 64 hex characters or base64 — generate one with `openssl rand -hex 32`")
}

func FromMap(m map[string]string) (*Config, error) {
	get := func(k string) string { return m[k] }
	bind := get("BIND")
	if bind == "" {
		bind = "127.0.0.1:8080"
	}
	if _, _, err := net.SplitHostPort(bind); err != nil {
		return nil, fmt.Errorf("BIND must be host:port, got %q", bind)
	}
	var secretKey []byte
	if raw := strings.TrimSpace(get("SECRET_KEY")); raw != "" {
		key, err := parseKey(raw)
		if err != nil {
			return nil, err
		}
		secretKey = key
	}
	publicURL := strings.TrimRight(get("PUBLIC_URL"), "/")
	dbPath := get("DB_PATH")
	if dbPath == "" {
		dbPath = "data/stock-predictor.db"
	}
	return &Config{DBPath: dbPath, Bind: bind, PublicURL: publicURL, SecretKey: secretKey}, nil
}

func FromEnv() (*Config, error) {
	m := map[string]string{}
	for _, kv := range os.Environ() {
		k, v, ok := strings.Cut(kv, "=")
		if ok {
			m[k] = v
		}
	}
	return FromMap(m)
}

// Retired variable names still present in the environment.
func RetiredWarnings() []string {
	out := []string{}
	for _, k := range Retired {
		if _, ok := os.LookupEnv(k); ok {
			out = append(out, k)
		}
	}
	sort.Strings(out)
	return out
}

// KeyPath is the fallback key file beside the database.
func (c *Config) KeyPath() string {
	dir := filepath.Dir(c.DBPath)
	if dir == "" {
		dir = "."
	}
	return filepath.Join(dir, "secret.key")
}
