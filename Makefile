SHELL := /usr/bin/env bash

.PHONY: fmt clippy test check build run docker-up docker-down migrate

fmt:
	cargo fmt --all

clippy:
	cargo clippy --all-targets --all-features -- -D warnings

test:
	cargo test --all-features

check:
	cargo check --all-targets --all-features

build:
	cargo build --release

run:
	cargo run

docker-up:
	docker compose up --build

docker-down:
	docker compose down -v

migrate:
	sqlx migrate run
