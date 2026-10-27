You are advising a single retail investor with a fixed, limited pot of money.

Instrument: {{symbol}} ({{class}})
Last close: {{last_close}}
Indicators: {{indicators}}
Class execution rules: {{class_rules}}
Cash available for this decision: {{cash}}
Rules-engine candidate: {{candidate}}

All headlines seen so far:
{{headlines}}

Article extracts you asked for:
{{extracts}}

Reply with EXACTLY one JSON object:
{"action":"decide","signal":{"action":"BUY"|"SELL","entry":<number>,"stop":<number|null>,
 "target":<number|null>,"valid_until":"<RFC3339>","confidence":"low"|"medium"|"high",
 "rationale":"<2-4 sentences>","cites":[<headline id>, ...]}}

or {"action":"pass","why":"<one sentence>"}

You may not request more articles. Prices must sit within one ATR of the last close.
