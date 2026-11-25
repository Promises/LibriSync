-- Licence pacing per Amazon customer, not per marketplace account.
--
-- Audible throttles a *customer*: the same Amazon login signed in for two marketplaces
-- ("amzn1.account.X:us" and "amzn1.account.X:uk") shares one throttle. Pacing state
-- (cooldown, spacing, the daily cap's request log, the "Audible unavailable" hold) is
-- therefore keyed by a pacing key: the Amazon customer id for Audible accounts, the
-- account id for every other provider. It is kept when an account is removed, so
-- removing and re-adding an account doesn't forget a cooldown.

ALTER TABLE account_state ADD COLUMN pacing_key TEXT;

-- Audible account ids are "{customer id}:{country code}". A state row whose account is
-- already gone from the core DB is recognised by Amazon's customer id prefix.
UPDATE account_state SET pacing_key = CASE
    WHEN COALESCE(
             (SELECT COALESCE(a.provider, 'audible') FROM core.Accounts a WHERE a.account_id = account_state.account_id),
             CASE WHEN account_id LIKE 'amzn1.account.%' THEN 'audible' ELSE 'unknown' END
         ) = 'audible'
         AND instr(account_id, ':') > 1
    THEN substr(account_id, 1, instr(account_id, ':') - 1)
    ELSE account_id
END;

CREATE TABLE licence_pacing (
    pacing_key TEXT PRIMARY KEY,
    -- Audible licence throttle cooldown.
    cooldown_until TEXT,
    cooldown_level INTEGER NOT NULL DEFAULT 0,
    cooldown_reason TEXT,
    -- Earliest next back-fill licence request (random spacing).
    next_licence_at TEXT,
    -- Short hold after Audible answered a licence request with a server or network
    -- error: no licence requests for the customer until then.
    hold_until TEXT,
    hold_reason TEXT
);

-- Two marketplaces of one customer merge: keep the stricter state.
INSERT INTO licence_pacing (pacing_key, cooldown_until, cooldown_level, cooldown_reason, next_licence_at)
SELECT pacing_key, MAX(cooldown_until), MAX(cooldown_level), MAX(cooldown_reason), MAX(next_licence_at)
FROM account_state
WHERE pacing_key IS NOT NULL
GROUP BY pacing_key;

ALTER TABLE account_state DROP COLUMN cooldown_until;
ALTER TABLE account_state DROP COLUMN cooldown_level;
ALTER TABLE account_state DROP COLUMN cooldown_reason;
ALTER TABLE account_state DROP COLUMN next_licence_at;

ALTER TABLE licence_requests ADD COLUMN pacing_key TEXT;
UPDATE licence_requests SET pacing_key = COALESCE(
    (SELECT s.pacing_key FROM account_state s WHERE s.account_id = licence_requests.account_id),
    account_id);
CREATE INDEX idx_licence_requests_key_time ON licence_requests(pacing_key, time);

-- The library folder a running item writes to, so two books whose folder template
-- renders the same path can't overwrite each other.
ALTER TABLE queue ADD COLUMN dest_path TEXT;

-- Successful syncs that could not set the baseline because they were incomplete (a
-- skipped page or an unreadable item). After a few, the baseline is set anyway.
ALTER TABLE account_state ADD COLUMN baseline_attempts INTEGER NOT NULL DEFAULT 0;
