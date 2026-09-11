ALTER TABLE threads ADD COLUMN thread_status TEXT;

UPDATE threads
SET thread_status = last_run_status
WHERE thread_status IS NULL
  AND last_run_status IS NOT NULL;
