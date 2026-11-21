package llm

import (
	"bytes"
	"crypto/sha256"
	"database/sql"
	"encoding/json"
	"fmt"
	"log/slog"
	"net/http"
	"sync"
	"time"

	"stock-predictor/internal/httpx"
	"stock-predictor/internal/llm/provider"
	"stock-predictor/internal/rules"
)

type HttpReply struct {
	Status int
	Body   string
}

// The one seam through which every outbound call goes, so tests never touch the network.
type Transport interface {
	Post(url, body string) (HttpReply, error)
}

type HTTPTransport struct {
	client *http.Client
	key    string
}

func NewHTTPTransport(key string, rt *rules.Runtime) (*HTTPTransport, error) {
	client, err := httpx.Client(rt, 60)
	if err != nil {
		return nil, err
	}
	return &HTTPTransport{client: client, key: key}, nil
}

func (t *HTTPTransport) Post(url, body string) (HttpReply, error) {
	req, err := http.NewRequest(http.MethodPost, url, bytes.NewReader([]byte(body)))
	if err != nil {
		return HttpReply{}, err
	}
	req.Header.Set("Authorization", "Bearer "+t.key)
	req.Header.Set("Content-Type", "application/json")
	resp, err := t.client.Do(req)
	if err != nil {
		return HttpReply{}, err
	}
	defer resp.Body.Close()
	var buf bytes.Buffer
	if _, err := buf.ReadFrom(resp.Body); err != nil {
		return HttpReply{}, err
	}
	return HttpReply{Status: resp.StatusCode, Body: buf.String()}, nil
}

// A queue of canned replies, plus the urls it was asked for.
type FixtureTransport struct {
	mu      sync.Mutex
	replies []HttpReply
	urls    []string
}

func NewFixtureTransport(replies []HttpReply) *FixtureTransport {
	return &FixtureTransport{replies: replies}
}

func OKFixture(body string) *FixtureTransport {
	return NewFixtureTransport([]HttpReply{{Status: 200, Body: body}})
}

func (f *FixtureTransport) Calls() int {
	f.mu.Lock()
	defer f.mu.Unlock()
	return len(f.urls)
}

func (f *FixtureTransport) Post(url, body string) (HttpReply, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.urls = append(f.urls, url)
	if len(f.replies) == 0 {
		return HttpReply{}, fmt.Errorf("fixture transport: no reply queued")
	}
	next := f.replies[0]
	f.replies = f.replies[1:]
	return next, nil
}

type ChatRequest struct {
	Trigger        *string
	ConversationID string
	Round          int64
	Prompt         string
	PromptTemplate string
	PromptHash     string
	Purpose        string
}

type ChatReply struct {
	Content          string
	PromptTokens     int64
	CompletionTokens int64
}

type SearchHit struct {
	Title   string  `json:"title"`
	Site    string  `json:"site"`
	URL     string  `json:"url"`
	Snippet string  `json:"snippet"`
	Summary string  `json:"summary"`
	Date    *string `json:"date"`
}

func Chat(db *sql.DB, model *provider.ModelConfig, transport Transport, req ChatRequest) (ChatReply, error) {
	body := string(model.RequestBody(req.Prompt))
	url := model.ChatURL()
	slog.Debug("llm request", "purpose", req.Purpose, "round", req.Round,
		"conversation", req.ConversationID, "prompt_chars", len(req.Prompt),
		"template", req.PromptTemplate)
	started := time.Now()
	reply, err := transport.Post(url, body)
	latency := time.Since(started).Milliseconds()

	round := req.Round
	row := ApiCallRow{
		ConversationID: &req.ConversationID,
		Round:          &round,
		Endpoint:       "chat",
		Purpose:        &req.Purpose,
		PromptTemplate: &req.PromptTemplate,
		PromptHash:     &req.PromptHash,
		RequestJSON:    &body,
		Trigger:        req.Trigger,
		LatencyMS:      &latency,
	}
	outcome, chatErr := parseChat(reply, err, model, &row)
	recordErr := RecordCall(db, row)
	if recordErr != nil {
		return outcome, recordErr
	}
	return outcome, chatErr
}

func parseChat(reply HttpReply, postErr error, model *provider.ModelConfig, row *ApiCallRow) (ChatReply, error) {
	if postErr != nil {
		e := postErr.Error()
		row.Error = &e
		return ChatReply{}, postErr
	}
	status := int64(reply.Status)
	row.HTTPStatus = &status
	row.ResponseJSON = &reply.Body

	if reply.Status < 200 || reply.Status > 299 {
		e := fmt.Sprintf("http %d", reply.Status)
		row.Error = &e
		return ChatReply{}, fmt.Errorf("deepseek chat: http %d", reply.Status)
	}

	var v map[string]any
	if err := json.Unmarshal([]byte(reply.Body), &v); err != nil {
		e := "malformed json: " + err.Error()
		row.Error = &e
		return ChatReply{}, fmt.Errorf("%s chat: malformed json: %w", model.Provider, err)
	}
	// Providers disagree on both the envelope and the error shape; both live in ModelConfig.
	if errVal, ok := v["error"]; ok {
		e := fmt.Sprint(errVal)
		row.Error = &e
		return ChatReply{}, fmt.Errorf("%s chat: %s", model.Provider, e)
	}
	content, ok := model.ExtractContent(v)
	if !ok {
		e := "response carried no assistant text"
		row.Error = &e
		return ChatReply{}, fmt.Errorf("%s chat: no assistant text", model.Provider)
	}
	promptTokens, completionTokens := model.Usage(v)
	row.PromptTokens = &promptTokens
	row.CompletionTokens = &completionTokens
	slog.Debug("llm reply", "status", reply.Status, "prompt_tokens", promptTokens,
		"completion_tokens", completionTokens, "chars", len(content))

	return ChatReply{Content: content, PromptTokens: promptTokens, CompletionTokens: completionTokens}, nil
}

func Search(db *sql.DB, base string, transport Transport, query string) ([]SearchHit, error) {
	return SearchFor(db, base, transport, query, query)
}

// The same call, with a name for the audit row. Purpose is what the audit page reads to
// say which holding a call was about, so a search for a stock is labelled with that stock.
func SearchFor(db *sql.DB, base string, transport Transport, query, purpose string) ([]SearchHit, error) {
	bodyBytes, _ := json.Marshal(map[string]any{
		"query": query,
		// Five is enough: the model picks a reason, and each hit costs prompt on every name.
		"count":        5,
		"need_content": false,
	})
	body := string(bodyBytes)
	url := trimRightSlash(base) + "/v1/tools/search"
	started := time.Now()
	reply, err := transport.Post(url, body)
	latency := time.Since(started).Milliseconds()

	row := ApiCallRow{
		Endpoint:    "search",
		Purpose:     &purpose,
		RequestJSON: &body,
		LatencyMS:   &latency,
	}
	hits, outcomeErr := parseSearch(reply, err, &row)
	if recordErr := RecordCall(db, row); recordErr != nil {
		return hits, recordErr
	}
	return hits, outcomeErr
}

func parseSearch(reply HttpReply, postErr error, row *ApiCallRow) ([]SearchHit, error) {
	if postErr != nil {
		e := postErr.Error()
		row.Error = &e
		return nil, postErr
	}
	status := int64(reply.Status)
	row.HTTPStatus = &status
	row.ResponseJSON = &reply.Body
	if reply.Status < 200 || reply.Status > 299 {
		e := fmt.Sprintf("http %d", reply.Status)
		row.Error = &e
		return nil, fmt.Errorf("deepseek search: http %d", reply.Status)
	}
	var v struct {
		Results []struct {
			Title    string  `json:"title"`
			SiteName string  `json:"site_name"`
			URL      string  `json:"url"`
			Snippet  string  `json:"snippet"`
			Summary  string  `json:"summary"`
			Date     *string `json:"date"`
		} `json:"results"`
	}
	if err := json.Unmarshal([]byte(reply.Body), &v); err != nil {
		e := "malformed json: " + err.Error()
		row.Error = &e
		return nil, fmt.Errorf("deepseek search: malformed json: %w", err)
	}
	out := make([]SearchHit, len(v.Results))
	for i, r := range v.Results {
		out[i] = SearchHit{
			Title: r.Title, Site: r.SiteName, URL: r.URL,
			Snippet: r.Snippet, Summary: r.Summary, Date: r.Date,
		}
	}
	return out, nil
}

func FetchPage(db *sql.DB, base string, transport Transport, urlToFetch string) (string, error) {
	bodyBytes, _ := json.Marshal(map[string]string{"url": urlToFetch})
	body := string(bodyBytes)
	url := trimRightSlash(base) + "/v1/tools/fetch"
	started := time.Now()
	reply, err := transport.Post(url, body)
	latency := time.Since(started).Milliseconds()

	row := ApiCallRow{
		Endpoint:    "fetch",
		Purpose:     &urlToFetch,
		RequestJSON: &body,
		LatencyMS:   &latency,
	}
	content, outcomeErr := parseFetch(reply, err, &row)
	if recordErr := RecordCall(db, row); recordErr != nil {
		return content, recordErr
	}
	return content, outcomeErr
}

func parseFetch(reply HttpReply, postErr error, row *ApiCallRow) (string, error) {
	if postErr != nil {
		e := postErr.Error()
		row.Error = &e
		return "", postErr
	}
	status := int64(reply.Status)
	row.HTTPStatus = &status
	row.ResponseJSON = &reply.Body
	if reply.Status < 200 || reply.Status > 299 {
		e := fmt.Sprintf("http %d", reply.Status)
		row.Error = &e
		return "", fmt.Errorf("deepseek fetch: http %d", reply.Status)
	}
	var v struct {
		Content string `json:"content"`
	}
	if err := json.Unmarshal([]byte(reply.Body), &v); err != nil {
		e := "malformed json: " + err.Error()
		row.Error = &e
		return "", fmt.Errorf("deepseek fetch: malformed json: %w", err)
	}
	return v.Content, nil
}

func trimRightSlash(s string) string {
	for len(s) > 0 && s[len(s)-1] == '/' {
		s = s[:len(s)-1]
	}
	return s
}

func sha256sum(s string) []byte {
	sum := sha256.Sum256([]byte(s))
	return sum[:]
}
