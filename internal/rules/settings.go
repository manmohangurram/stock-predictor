package rules

import (
	"database/sql"
	"fmt"
	"log/slog"
	"math"
	"os"
	"strconv"
	"strings"
	"time"

	"stock-predictor/internal/config"
	"stock-predictor/internal/llm/provider"
	"stock-predictor/internal/models"
	"stock-predictor/internal/proxyurl"
	"stock-predictor/internal/secrets"
	"stock-predictor/internal/tstamp"
)

type Settings struct {
	Budget              float64 `json:"budget"`
	Committed           float64 `json:"committed"`
	Available           float64 `json:"available"`
	Currency            string  `json:"currency"`
	CurrencyLocked      bool    `json:"currency_locked"`
	DeepseekConfigured  bool    `json:"deepseek_configured"`
	TradingBudget       float64 `json:"trading_budget"`
	TradingAvailable    float64 `json:"trading_available"`
	MfBudget            float64 `json:"mf_budget"`
	MfAvailable         float64 `json:"mf_available"`
	NtfyURL             string  `json:"ntfy_url"`
	NtfyTopic           string  `json:"ntfy_topic"`
	NtfyTokenConfigured bool    `json:"ntfy_token_configured"`
	LlmProvider         string  `json:"llm_provider"`
	LlmModel            string  `json:"llm_model"`
	LlmBaseURL          string  `json:"llm_base_url"`
	LlmKeyConfigured    bool    `json:"llm_key_configured"`
	LimitsLong          Limits  `json:"limits_long"`
	LimitsTrade         Limits  `json:"limits_trade"`
	Runtime             View    `json:"runtime"`
}

type SettingsPatch struct {
	Budget            *float64 `json:"budget"`
	Currency          *string  `json:"currency"`
	DeepseekKey       *string  `json:"deepseek_key"`
	TradingBudget     *float64 `json:"trading_budget"`
	MfBudget          *float64 `json:"mf_budget"`
	NtfyURL           *string  `json:"ntfy_url"`
	NtfyTopic         *string  `json:"ntfy_topic"`
	NtfyToken         *string  `json:"ntfy_token"`
	LlmProvider       *string  `json:"llm_provider"`
	LlmModel          *string  `json:"llm_model"`
	LlmBaseURL        *string  `json:"llm_base_url"`
	LlmKey            *string  `json:"llm_key"`
	LimitsLong        *Limits  `json:"limits_long"`
	LimitsTrade       *Limits  `json:"limits_trade"`
	PollSecs          *uint64  `json:"poll_secs"`
	OffHoursSecs      *uint64  `json:"off_hours_secs"`
	PromptWindowHours *uint32  `json:"prompt_window_hours"`
	MinPriceMove      *float64 `json:"min_price_move"`
	DailyCap          *uint32  `json:"daily_cap"`
	ScanMinPrice      *float64 `json:"scan_min_price"`
	ScanMinTurnover   *float64 `json:"scan_min_turnover"`
	ScanLockedPct     *float64 `json:"scan_locked_pct"`
	Proxy             *string  `json:"proxy"`
	ProxyBypass       *string  `json:"proxy_bypass"`
	ProxyUser         *string  `json:"proxy_user"`
	ProxyPass         *string  `json:"proxy_pass"`
}

type SettingsErrorCode string

const (
	ErrCurrencyLocked SettingsErrorCode = "currency is fixed once the ledger has entries"
	ErrInvalidAmount  SettingsErrorCode = "invalid amount"
)

type SettingsError struct {
	Code    SettingsErrorCode
	Message string
}

func (e SettingsError) Error() string {
	if e.Message != "" {
		return e.Message
	}
	return string(e.Code)
}

// 900.0 shows as 900, so the settings table stays readable in a query.
func TrimNumber(v float64) string {
	if math.Abs(v-math.Trunc(v)) < 1e-9 {
		return strconv.FormatInt(int64(v), 10)
	}
	return strconv.FormatFloat(v, 'g', -1, 64)
}

func GetSettings(pool *sql.DB) (*Settings, error) {
	budget, err := allowanceSetting(pool, models.ModeLong)
	if err != nil {
		return nil, err
	}
	committed, err := Committed(pool)
	if err != nil {
		return nil, err
	}
	currency, err := getCurrency(pool)
	if err != nil {
		return nil, err
	}
	locked, err := ledgerHasEntries(pool)
	if err != nil {
		return nil, err
	}
	dsKey, err := DeepseekKey(pool)
	if err != nil {
		return nil, err
	}
	tradingBudget, err := allowanceSetting(pool, models.ModeTrade)
	if err != nil {
		return nil, err
	}
	mfBudget, err := MFAllowance(pool)
	if err != nil {
		return nil, err
	}
	tradingAvailable, err := AvailableIn(pool, models.ModeTrade)
	if err != nil {
		return nil, err
	}
	ntfyTopic, _ := Setting(pool, "ntfy_topic")
	ntfyURL, err := Setting(pool, "ntfy_url")
	if err != nil {
		return nil, err
	}
	if ntfyURL == nil {
		def := "https://ntfy.sh"
		ntfyURL = &def
	}
	ntfyTokenConfigured := false
	if tok, err := Setting(pool, "ntfy_token"); err == nil && tok != nil && strings.TrimSpace(*tok) != "" {
		ntfyTokenConfigured = true
	}
	providerRaw, err := Setting(pool, "llm_provider")
	if err != nil {
		return nil, err
	}
	prov, perr := provider.ParseProvider(orDefault(providerRaw, ""))
	if perr != nil {
		prov = provider.Deepseek
	}
	llmModel, err := Setting(pool, "llm_model")
	if err != nil {
		return nil, err
	}
	if llmModel == nil || *llmModel == "" {
		def := prov.DefaultModel()
		llmModel = &def
	}
	llmBaseURL, err := Setting(pool, "llm_base_url")
	if err != nil {
		return nil, err
	}
	if llmBaseURL == nil || *llmBaseURL == "" {
		def := prov.DefaultBaseURL()
		llmBaseURL = &def
	}
	llmKeyConfigured := false
	if k, err := KeyFor(pool, prov); err == nil && k != nil {
		llmKeyConfigured = true
	}
	limitsLong, err := LimitsFor(pool, models.ModeLong)
	if err != nil {
		return nil, err
	}
	limitsTrade, err := LimitsFor(pool, models.ModeTrade)
	if err != nil {
		return nil, err
	}
	rt, err := LoadRuntime(pool)
	if err != nil {
		return nil, SettingsError{Code: ErrInvalidAmount}
	}
	mfAvailable, err := AvailableMF(pool)
	if err != nil {
		mfAvailable = 0.0
	}
	available, err := AvailableIn(pool, models.ModeLong)
	if err != nil {
		return nil, err
	}
	return &Settings{
		Budget:              budget,
		Committed:           committed,
		Available:           available,
		Currency:            currency,
		CurrencyLocked:      locked,
		DeepseekConfigured:  dsKey != nil,
		TradingBudget:       tradingBudget,
		TradingAvailable:    tradingAvailable,
		MfBudget:            mfBudget,
		MfAvailable:         mfAvailable,
		NtfyURL:             *ntfyURL,
		NtfyTopic:           orDefault(ntfyTopic, ""),
		NtfyTokenConfigured: ntfyTokenConfigured,
		LlmProvider:         string(prov),
		LlmModel:            *llmModel,
		LlmBaseURL:          *llmBaseURL,
		LlmKeyConfigured:    llmKeyConfigured,
		LimitsLong:          limitsLong,
		LimitsTrade:         limitsTrade,
		Runtime:             rt.View(),
	}, nil
}

func orDefault(p *string, def string) string {
	if p == nil {
		return def
	}
	return *p
}

func SetSettings(pool *sql.DB, patch SettingsPatch) error {
	for _, kv := range []struct {
		key string
		v   *float64
	}{
		{"poll_secs", f64FromU64(patch.PollSecs)},
		{"off_hours_secs", f64FromU64(patch.OffHoursSecs)},
		{"prompt_window_hours", f64FromU32(patch.PromptWindowHours)},
		{"min_price_move", patch.MinPriceMove},
		{"daily_cap", f64FromU32x(patch.DailyCap)},
		{"scan_min_price", patch.ScanMinPrice},
		{"scan_min_turnover", patch.ScanMinTurnover},
		{"scan_locked_pct", patch.ScanLockedPct},
	} {
		if kv.v == nil {
			continue
		}
		if err := RuntimeCheck(kv.key, *kv.v); err != nil {
			return SettingsError{Message: err.Error()}
		}
		if err := putSetting(pool, kv.key, TrimNumber(*kv.v)); err != nil {
			return SettingsError{Code: ErrInvalidAmount}
		}
	}

	if patch.Proxy != nil {
		proxy := strings.TrimSpace(*patch.Proxy)
		if proxy == "" {
			for _, key := range []string{"proxy", "proxy_user", "proxy_pass"} {
				if _, err := pool.Exec("DELETE FROM settings WHERE key = ?", key); err != nil {
					return SettingsError{Code: ErrInvalidAmount}
				}
			}
		} else {
			if err := proxyurl.ValidateProxy(proxy); err != nil {
				return SettingsError{Message: err.Error()}
			}
			if err := putSetting(pool, "proxy", proxy); err != nil {
				return SettingsError{Code: ErrInvalidAmount}
			}
		}
	}

	if patch.ProxyUser != nil {
		if err := putSetting(pool, "proxy_user", strings.TrimSpace(*patch.ProxyUser)); err != nil {
			return SettingsError{Code: ErrInvalidAmount}
		}
	}

	if patch.ProxyPass != nil {
		if pass := strings.TrimSpace(*patch.ProxyPass); pass != "" {
			if err := secrets.PutSecret(pool, "proxy_pass", pass); err != nil {
				return SettingsError{Code: ErrInvalidAmount}
			}
		}
	}

	if patch.ProxyBypass != nil {
		if err := putSetting(pool, "proxy_bypass", strings.TrimSpace(*patch.ProxyBypass)); err != nil {
			return SettingsError{Code: ErrInvalidAmount}
		}
	}

	setAllowance := func(key string, mode models.Mode, bucket string, budget *float64) error {
		if budget == nil {
			return nil
		}
		if math.IsNaN(*budget) || math.IsInf(*budget, 0) || *budget < 0.0 {
			return SettingsError{Code: ErrInvalidAmount}
		}
		if err := putSetting(pool, key, TrimNumber(*budget)); err != nil {
			return SettingsError{Code: ErrInvalidAmount}
		}
		if _, err := EnsureMonth(pool, mode, bucket, *budget, time.Now().UTC()); err != nil {
			return SettingsError{Code: ErrInvalidAmount}
		}
		return nil
	}
	if err := setAllowance("allowance_long", models.ModeLong, "", patch.Budget); err != nil {
		return err
	}
	if err := setAllowance("allowance_long_mf", models.ModeLong, "mf", patch.MfBudget); err != nil {
		return err
	}
	// Apply it now, not on the 1st: setting an allowance should make it spendable.
	if err := setAllowance("allowance_trade", models.ModeTrade, "", patch.TradingBudget); err != nil {
		return err
	}

	if patch.Currency != nil {
		locked, err := ledgerHasEntries(pool)
		if err != nil {
			return SettingsError{Code: ErrInvalidAmount}
		}
		if locked {
			return SettingsError{Code: ErrCurrencyLocked}
		}
		if _, err := pool.Exec(
			"INSERT INTO settings (key, value, updated_at) VALUES ('currency', ?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
			*patch.Currency, tstamp.Now(),
		); err != nil {
			return SettingsError{Code: ErrInvalidAmount}
		}
	}

	for _, kv := range []struct {
		name string
		v    *string
	}{
		{"ntfy_url", patch.NtfyURL},
		{"ntfy_topic", patch.NtfyTopic},
	} {
		if kv.v != nil {
			// An empty string is a deliberate clear for these; the token is different.
			if err := putSetting(pool, kv.name, strings.TrimSpace(*kv.v)); err != nil {
				return SettingsError{Code: ErrInvalidAmount}
			}
		}
	}

	for _, kv := range []struct {
		mode   models.Mode
		limits *Limits
	}{
		{models.ModeLong, patch.LimitsLong},
		{models.ModeTrade, patch.LimitsTrade},
	} {
		if kv.limits == nil {
			continue
		}
		l := *kv.limits
		if math.IsNaN(l.MinProfitPct) || math.IsInf(l.MinProfitPct, 0) ||
			math.IsNaN(l.MaxProfitPct) || math.IsInf(l.MaxProfitPct, 0) ||
			l.MinProfitPct < 0.0 || l.MaxProfitPct < l.MinProfitPct ||
			l.MinTradeAmount < 0.0 || l.MaxTradeAmount < l.MinTradeAmount ||
			!(l.MaxInstrumentPct >= 0.0 && l.MaxInstrumentPct <= 100.0) {
			return SettingsError{Code: ErrInvalidAmount}
		}
		for _, name := range []string{
			"min_profit_pct", "max_profit_pct", "min_trade_amount",
			"max_trade_amount", "max_instrument_pct",
		} {
			if value, ok := l.field(name); ok {
				if err := putSetting(
					pool, fmt.Sprintf("%s_%s", LimitsKeyPrefix(kv.mode), name),
					strconv.FormatFloat(value, 'g', -1, 64),
				); err != nil {
					return SettingsError{Code: ErrInvalidAmount}
				}
			}
		}
	}

	if patch.LlmProvider != nil {
		p, err := provider.ParseProvider(*patch.LlmProvider)
		if err != nil {
			return SettingsError{Code: ErrInvalidAmount}
		}
		if err := putSetting(pool, "llm_provider", string(p)); err != nil {
			return SettingsError{Code: ErrInvalidAmount}
		}
	}
	for _, kv := range []struct {
		name string
		v    *string
	}{
		{"llm_model", patch.LlmModel},
		{"llm_base_url", patch.LlmBaseURL},
	} {
		if kv.v != nil {
			if err := putSetting(pool, kv.name, strings.TrimSpace(*kv.v)); err != nil {
				return SettingsError{Code: ErrInvalidAmount}
			}
		}
	}
	if patch.LlmKey != nil {
		if key := strings.TrimSpace(*patch.LlmKey); key != "" {
			providerRaw, err := Setting(pool, "llm_provider")
			if err != nil {
				return SettingsError{Code: ErrInvalidAmount}
			}
			prov, _ := provider.ParseProvider(orDefault(providerRaw, ""))
			if err := secrets.PutSecret(pool, KeyName(prov), key); err != nil {
				return SettingsError{Code: ErrInvalidAmount}
			}
		}
	}

	if patch.NtfyToken != nil {
		if token := strings.TrimSpace(*patch.NtfyToken); token != "" {
			if err := secrets.PutSecret(pool, "ntfy_token", token); err != nil {
				return SettingsError{Code: ErrInvalidAmount}
			}
		}
	}

	if patch.DeepseekKey != nil {
		if key := strings.TrimSpace(*patch.DeepseekKey); key != "" {
			if err := putSetting(pool, "deepseek_key", key); err != nil {
				return SettingsError{Code: ErrInvalidAmount}
			}
		}
	}

	return nil
}

func f64FromU64(v *uint64) *float64 {
	if v == nil {
		return nil
	}
	f := float64(*v)
	return &f
}

func f64FromU32(v *uint32) *float64 {
	if v == nil {
		return nil
	}
	f := float64(*v)
	return &f
}

func f64FromU32x(v *uint32) *float64 { return f64FromU32(v) }

// The configured monthly allowance: what the month reset grants on the 1st.
func Allowance(pool *sql.DB, mode models.Mode) (float64, error) {
	return allowanceSetting(pool, mode)
}

func allowanceSetting(pool *sql.DB, mode models.Mode) (float64, error) {
	key := "allowance_long"
	if mode == models.ModeTrade {
		key = "allowance_trade"
	}
	v, err := Setting(pool, key)
	if err != nil {
		return 0, err
	}
	if v == nil {
		return 0.0, nil
	}
	f, err := strconv.ParseFloat(strings.TrimSpace(*v), 64)
	if err != nil || math.IsNaN(f) || math.IsInf(f, 0) || f < 0.0 {
		return 0.0, nil
	}
	return f, nil
}

// The monthly allowance earmarked for mutual funds, inside the long-term pot.
func MFAllowance(pool *sql.DB) (float64, error) {
	v, err := Setting(pool, "allowance_long_mf")
	if err != nil {
		return 0, err
	}
	if v == nil {
		return 0.0, nil
	}
	f, err := strconv.ParseFloat(strings.TrimSpace(*v), 64)
	if err != nil || math.IsNaN(f) || math.IsInf(f, 0) || f < 0.0 {
		return 0.0, nil
	}
	return f, nil
}

// The key itself, for the client that actually calls the model. Never exposed over HTTP.
func Setting(pool *sql.DB, key string) (*string, error) {
	var value string
	err := pool.QueryRow("SELECT value FROM settings WHERE key = ?", key).Scan(&value)
	if err == sql.ErrNoRows || (err == nil && value == "") {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	return &value, nil
}

// The token itself, for the notifier. Never serialised.
func NtfyToken(pool *sql.DB) (*string, error) {
	s, err := secrets.GetSecret(pool, "ntfy_token")
	if err != nil || s == "" {
		return nil, err
	}
	return &s, nil
}

func KeyName(p provider.Provider) string {
	switch p {
	case provider.Openai:
		return "openai_key"
	case provider.Gemini:
		return "gemini_key"
	case provider.Custom:
		return "custom_key"
	}
	return "deepseek_key"
}

// The key for one provider. The database wins; a developer's .env is the fallback, so a
// prompt can be exercised locally without storing a key at rest. Nothing read from the
// environment is ever written back to a setting.
func KeyFor(pool *sql.DB, p provider.Provider) (*string, error) {
	stored, err := secrets.GetSecret(pool, KeyName(p))
	if err != nil {
		return nil, err
	}
	if stored != "" && strings.TrimSpace(stored) != "" {
		return &stored, nil
	}
	if env := os.Getenv(config.KeyEnvName(string(p))); strings.TrimSpace(env) != "" {
		return &env, nil
	}
	return nil, nil
}

// The limits for one pot, defaulted field by field so a partial configuration still works.
func LimitsFor(pool *sql.DB, mode models.Mode) (Limits, error) {
	limits := DefaultLimits()
	for _, name := range []string{
		"min_profit_pct", "max_profit_pct", "min_trade_amount",
		"max_trade_amount", "max_instrument_pct",
	} {
		key := fmt.Sprintf("%s_%s", LimitsKeyPrefix(mode), name)
		v, err := Setting(pool, key)
		if err != nil || v == nil {
			continue
		}
		if value, err := strconv.ParseFloat(*v, 64); err == nil {
			if next, ok := limits.With(name, value); ok {
				limits = next
			}
		}
	}
	return limits, nil
}

// Everything a chat call needs, resolved from the settings table.
func ModelConfigFor(pool *sql.DB) (provider.ModelConfig, error) {
	providerRaw, err := Setting(pool, "llm_provider")
	if err != nil {
		return provider.ModelConfig{}, err
	}
	prov, _ := provider.ParseProvider(orDefault(providerRaw, ""))
	model, err := Setting(pool, "llm_model")
	if err != nil {
		return provider.ModelConfig{}, err
	}
	if model == nil || *model == "" {
		def := prov.DefaultModel()
		model = &def
	}
	baseURL, err := Setting(pool, "llm_base_url")
	if err != nil {
		return provider.ModelConfig{}, err
	}
	if baseURL == nil || *baseURL == "" {
		def := prov.DefaultBaseURL()
		baseURL = &def
	}
	apiKey := ""
	if k, err := KeyFor(pool, prov); err == nil && k != nil {
		apiKey = *k
	}
	return provider.ModelConfig{
		Provider: prov,
		Model:    *model,
		BaseURL:  *baseURL,
		APIKey:   apiKey,
	}, nil
}

func DeepseekKey(pool *sql.DB) (*string, error) {
	return KeyFor(pool, provider.Deepseek)
}

func putSetting(pool *sql.DB, key, value string) error {
	slog.Debug("setting written", "key", key)
	_, err := pool.Exec(
		"INSERT INTO settings (key, value, updated_at) VALUES (?, ?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
		key, value, tstamp.Now(),
	)
	return err
}

// First boot only: make sure the currency row exists. Everything else — budget, trading
// pot, keys, ntfy — starts empty and is set in the UI, so there is exactly one source of
// truth.
func Seed(pool *sql.DB) error {
	var n int
	if err := pool.QueryRow("SELECT COUNT(*) FROM settings WHERE key = 'currency'").Scan(&n); err != nil {
		return err
	}
	if n == 0 {
		return putSetting(pool, "currency", "INR")
	}
	return nil
}

func getCurrency(pool *sql.DB) (string, error) {
	var value string
	err := pool.QueryRow("SELECT value FROM settings WHERE key = 'currency'").Scan(&value)
	if err == sql.ErrNoRows {
		return "INR", nil
	}
	if err != nil {
		return "", err
	}
	return value, nil
}

func ledgerHasEntries(pool *sql.DB) (bool, error) {
	var n int
	if err := pool.QueryRow("SELECT COUNT(*) FROM cash_ledger WHERE kind <> 'budget'").Scan(&n); err != nil {
		return false, err
	}
	return n > 0, nil
}
