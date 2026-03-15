#!/bin/bash

FILENAME=$(date +"%d_%m_%Y.txt")
cargo run --release |& tee -a "$FILENAME"
