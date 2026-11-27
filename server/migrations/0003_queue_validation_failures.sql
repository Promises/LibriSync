-- How many attempts of a queue item failed validation (the downloaded source didn't
-- decode). Every retry re-plans, which for Audible is another licence request, so a
-- source that fails validation is retried once, not like a network blip.
ALTER TABLE queue ADD COLUMN validation_failures INTEGER NOT NULL DEFAULT 0;
