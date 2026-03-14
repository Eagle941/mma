#!/bin/bash

# 1. Check for the required endpoint argument
if [ -z "$1" ]; then
    echo "Usage: $0 <endpoint>"
    echo "Example: $0 '/v5/account/wallet-balance?accountType=UNIFIED'"
    exit 1
fi

ENDPOINT="$1"
BASE_URL="https://api-testnet.bybit.com"

# 2. Extract API credentials from the .secrets file safely
if [ ! -f ".secrets" ]; then
    echo "Error: .secrets file not found in the current directory."
    exit 1
fi

# Extract variables, stripping out any single/double quotes just in case
API_KEY=$(grep -E '^API_KEY=' .secrets | cut -d '=' -f 2 | tr -d '"' | tr -d "'")
API_SECRET=$(grep -E '^API_SECRET=' .secrets | cut -d '=' -f 2 | tr -d '"' | tr -d "'")

if [ -z "$API_KEY" ] || [ -z "$API_SECRET" ]; then
    echo "Error: API_KEY or API_SECRET not correctly defined in .secrets file."
    exit 1
fi

# 3. Generate timestamp and set Recv Window
RECV_WINDOW=1000
TIMESTAMP=$(date +%s%3N) # Generates epoch time in milliseconds

# 4. Extract query string for the signature (Bybit requires this for GET requests)
if [[ "$ENDPOINT" == *"?"* ]]; then
    QUERY_STRING="${ENDPOINT#*\?}"
else
    QUERY_STRING=""
fi

# 5. Build the signature string
# Bybit V5 Rule: timestamp + api_key + recv_window + queryString
SIGN_STR="${TIMESTAMP}${API_KEY}${RECV_WINDOW}${QUERY_STRING}"

# 6. Generate the HMAC-SHA256 signature
# Openssl outputs "(stdin)= hash", so we use sed to isolate just the hash
SIGNATURE=$(echo -n "$SIGN_STR" | openssl dgst -sha256 -hmac "$API_SECRET" | sed 's/^.* //')

# 7. Execute the GET request
curl -s -X GET "${BASE_URL}${ENDPOINT}" \
     -H "X-BAPI-API-KEY: ${API_KEY}" \
     -H "X-BAPI-TIMESTAMP: ${TIMESTAMP}" \
     -H "X-BAPI-SIGN: ${SIGNATURE}" \
     -H "X-BAPI-RECV-WINDOW: ${RECV_WINDOW}" | jq

echo "" # Add a newline after the JSON response for readability
