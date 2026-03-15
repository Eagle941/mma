#!/bin/bash

FILENAME=$(date +"%d_%m_%Y.log")
cargo run --release |& tee -a "$FILENAME"
