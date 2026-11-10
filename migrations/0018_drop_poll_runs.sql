-- A scheduled poll wrote a run row every tick: ninety-six "nothing happened" rows a day, none of
-- them shown anywhere. The calls a poll makes are already in `api_calls`; the three prompts that
-- decide something keep their rows. Manual polls still record, because a person asked.
DELETE FROM runs WHERE prompt = 'poll' AND trigger = 'schedule';
