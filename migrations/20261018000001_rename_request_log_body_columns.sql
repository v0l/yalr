ALTER TABLE request_log_body RENAME COLUMN client_request_gz TO client_request_packed;
ALTER TABLE request_log_body RENAME COLUMN upstream_request_gz TO upstream_request_packed;
ALTER TABLE request_log_body RENAME COLUMN response_gz TO response_packed;
