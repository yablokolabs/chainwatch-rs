CREATE TABLE IF NOT EXISTS chains (
    chain_id BIGINT PRIMARY KEY,
    name TEXT NOT NULL,
    rpc_url_redacted TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS blocks (
    chain_id BIGINT NOT NULL REFERENCES chains(chain_id) ON DELETE CASCADE,
    number BIGINT NOT NULL,
    hash TEXT NOT NULL,
    parent_hash TEXT NOT NULL,
    timestamp BIGINT NOT NULL,
    tx_count BIGINT NOT NULL,
    inserted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (chain_id, number),
    UNIQUE (chain_id, hash)
);

CREATE TABLE IF NOT EXISTS transactions (
    chain_id BIGINT NOT NULL REFERENCES chains(chain_id) ON DELETE CASCADE,
    hash TEXT NOT NULL,
    block_number BIGINT NOT NULL,
    tx_index BIGINT NOT NULL,
    from_address TEXT NOT NULL,
    to_address TEXT,
    value_wei TEXT NOT NULL,
    input BYTEA NOT NULL,
    status BOOLEAN,
    gas_used BIGINT,
    inserted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (chain_id, hash),
    FOREIGN KEY (chain_id, block_number) REFERENCES blocks(chain_id, number) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS logs (
    chain_id BIGINT NOT NULL REFERENCES chains(chain_id) ON DELETE CASCADE,
    block_number BIGINT NOT NULL,
    tx_hash TEXT NOT NULL,
    log_index BIGINT NOT NULL,
    address TEXT NOT NULL,
    topics JSONB NOT NULL,
    data BYTEA NOT NULL,
    removed BOOLEAN NOT NULL DEFAULT false,
    timestamp BIGINT NOT NULL,
    inserted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (chain_id, tx_hash, log_index),
    FOREIGN KEY (chain_id, tx_hash) REFERENCES transactions(chain_id, hash) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS decoded_events (
    id BIGSERIAL PRIMARY KEY,
    chain_id BIGINT NOT NULL REFERENCES chains(chain_id) ON DELETE CASCADE,
    event_type TEXT NOT NULL,
    token_address TEXT NOT NULL,
    tx_hash TEXT NOT NULL,
    block_number BIGINT NOT NULL,
    log_index BIGINT NOT NULL,
    timestamp BIGINT NOT NULL,
    payload JSONB NOT NULL,
    inserted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (chain_id, tx_hash, log_index, event_type),
    FOREIGN KEY (chain_id, tx_hash, log_index) REFERENCES logs(chain_id, tx_hash, log_index) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS token_transfers (
    chain_id BIGINT NOT NULL REFERENCES chains(chain_id) ON DELETE CASCADE,
    token_address TEXT NOT NULL,
    from_address TEXT NOT NULL,
    to_address TEXT NOT NULL,
    amount_wei TEXT NOT NULL,
    tx_hash TEXT NOT NULL,
    block_number BIGINT NOT NULL,
    log_index BIGINT NOT NULL,
    timestamp BIGINT NOT NULL,
    inserted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (chain_id, tx_hash, log_index),
    FOREIGN KEY (chain_id, tx_hash, log_index) REFERENCES logs(chain_id, tx_hash, log_index) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS watchlist (
    chain_id BIGINT NOT NULL REFERENCES chains(chain_id) ON DELETE CASCADE,
    address TEXT NOT NULL,
    label TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (chain_id, address)
);

CREATE TABLE IF NOT EXISTS alerts (
    id UUID PRIMARY KEY,
    chain_id BIGINT NOT NULL REFERENCES chains(chain_id) ON DELETE CASCADE,
    rule TEXT NOT NULL,
    severity TEXT NOT NULL CHECK (severity IN ('low', 'medium', 'high', 'critical')),
    address TEXT,
    tx_hash TEXT,
    block_number BIGINT,
    message TEXT NOT NULL,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    acknowledged_at TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS indexer_state (
    chain_id BIGINT PRIMARY KEY REFERENCES chains(chain_id) ON DELETE CASCADE,
    latest_block BIGINT,
    latest_hash TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_blocks_chain_hash ON blocks(chain_id, hash);
CREATE INDEX IF NOT EXISTS idx_transactions_chain_block ON transactions(chain_id, block_number DESC, tx_index DESC);
CREATE INDEX IF NOT EXISTS idx_transactions_from ON transactions(chain_id, from_address);
CREATE INDEX IF NOT EXISTS idx_transactions_to ON transactions(chain_id, to_address);
CREATE INDEX IF NOT EXISTS idx_logs_address_block ON logs(chain_id, address, block_number DESC);
CREATE INDEX IF NOT EXISTS idx_logs_topics_gin ON logs USING GIN (topics);
CREATE INDEX IF NOT EXISTS idx_decoded_events_type_block ON decoded_events(chain_id, event_type, block_number DESC);
CREATE INDEX IF NOT EXISTS idx_token_transfers_address ON token_transfers(chain_id, from_address, block_number DESC);
CREATE INDEX IF NOT EXISTS idx_token_transfers_to ON token_transfers(chain_id, to_address, block_number DESC);
CREATE INDEX IF NOT EXISTS idx_token_transfers_token ON token_transfers(chain_id, token_address, block_number DESC);
CREATE INDEX IF NOT EXISTS idx_alerts_chain_created ON alerts(chain_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_alerts_rule ON alerts(chain_id, rule, created_at DESC);
