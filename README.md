# Market MAker

## Architecture

Courtesy of DeepSeek

This diagram represents a simplified, high-level architecture. Real-world
systems are far more complex and have additional layers for risk management and
monitoring.

### Block Diagram of a Market Making Bot

```text
+-----------------------------------------------------------------------+
|                        MARKET MAKING BOT                              |
|                                                                       |
|  +-------------------+                                                |
|  |  Configuration &  |                                                |
|  |  Strategy Engine  |                                                |
|  +-------------------+                                                |
|         |                                                             |
|         v                                                             |
|  +-------------------+      +-------------------+                     |
|  |  Pricing & Quote  | <--> |  Risk & Inventory |                     |
|  |  Engine           |      |  Management       |                     |
|  +-------------------+      +-------------------+                     |
|         | ^                            |                              |
|         v |                            v                              |
|  +-------------------+      +-------------------+                     |
|  |  Order Management |      |  Hedging Engine   |                     |
|  |  System (OMS)     |      |  (Optional)       |                     |
|  +-------------------+      +-------------------+                     |
|         |                              |                              |
+---------|------------------------------|------------------------------+
          |                              |
          v                              v
+---------|------------------------------|------------------------------+
|                        EXCHANGE CONNECTION LAYER                      |
|                                                                       |
|  +-------------------+        +-------------------+                   |
|  |  Market Data      |        |  Order Execution  |                   |
|  |  Feed Handler     |        |  Handler          |                   |
|  +-------------------+        +-------------------+                   |
|         | ^                              | ^                          |
+---------|--------------------------------|----------------------------+
          |                                |
          v                                v
+---------|--------------------------------|----------------------------+
|                              EXCHANGE(S)                              |
|                                                                       |
|          +-----------------------------+                              |
|          | Order Books, Trades, etc.   |                              |
|          +-----------------------------+                              |
+-----------------------------------------------------------------------+
```

### Explanation of Each Block
Let's break down what each component does:

1. Configuration & Strategy Engine
    This is the brain that defines how the bot will market make.

    __Inputs__: Human-defined parameters (e.g., spread width, order size,
    maximum inventory, target symbols).

    __Function__: Calculates the core logic for bid/ask prices and sizes based
    on the chosen strategy (e.g., fixed spread, inventory-sensitive,
    volatility-adjusted).

2. Pricing & Quote Engine
    Continuously calculates the exact bid and ask prices to post. It uses the
    market's mid-price and applies the defined spread and skew (e.g., if
    inventory is long, lower the bid price to discourage buying).

    __Inputs__: Live market data from the exchange and parameters from the
    Strategy Engine.

    __Outputs__: The desired quotes (price, size, side) to be sent to the
    market.

3. Risk & Inventory Management
    Monitors the bot's exposure. It tracks how much of an asset the bot holds
    and calculates the associated risk. If the inventory exceeds a predefined
    limit, it signals the Pricing Engine to adjust quotes to reduce the
    position (e.g., sell more aggressively if too long).

    __Inputs__: Current positions (inventory), PnL, and filled orders from the
    OMS.

4. Order Management System (OMS)
    Manages the lifecycle of all orders. It sends new orders, cancels old ones,
    and updates existing orders as market conditions change. It ensures the
    bot's intentions are accurately reflected in the order book.

    __Inputs__: Quotes from the Pricing Engine.

5. Hedging Engine (Optional but Critical for FX/CFDs)
    In markets where it's possible (e.g., using futures or a different venue),
    this engine immediately executes an offsetting trade to "hedge" the
    inventory risk and lock in the spread. For example, if a bot sells BTC on
    the spot market, it might buy a BTC perpetual futures contract to become
    delta-neutral.

    __Inputs__: Filled orders from the OMS (e.g., the bot just sold 1 BTC, so it
    now has a short BTC position).

6. Exchange Connection Layer
    Market Data Feed Handler:

    __Function__: Establishes a low-latency connection to the exchange's data
    feed (often via WebSocket). It receives real-time updates for order books,
    recent trades, and ticker information, then parses and normalizes this data
    for the Pricing Engine.

    Order Execution Handler:

    __Function__: Establishes a secure connection (using API keys) to send
    orders, cancel orders, and check the status of orders. It receives
    confirmations (fills, cancellations, errors) from the exchange and relays
    them back to the OMS and Risk Management modules.

7. Exchange(s)
    The external venue where the trading occurs. It provides the market data and
    executes the orders sent by the bot.

### Data Flow Explained
Market Data In: The exchange sends live market data (e.g., the latest order book
snapshot) to the Market Data Feed Handler.

Price Calculation: The handler passes this data to the Pricing & Quote Engine.
The engine, guided by the Strategy Engine and current risk from Risk & Inventory
Management, calculates new bid/ask quotes.

Order Submission: The new quotes are sent to the OMS, which decides to send,
cancel, or update orders. The Order Execution Handler transmits these commands
to the exchange.

Fill & Feedback: The exchange executes a trade (a "fill") against the bot's
order. The fill confirmation is sent back through the Order Execution Handler to
the OMS.

Risk Update: The OMS notifies the Risk & Inventory Management module of the
fill, updating the bot's inventory and PnL.

Hedging (Optional): The Risk module may trigger the Hedging Engine to execute an
offsetting trade on another market to neutralize the newly acquired risk.

Loop: This entire process runs in a continuous loop, often millions of times per
day, with the bot constantly adjusting its quotes in response to the market.

## Instructions

Before the library can be compiled on Linux, make sure the following
dependencies have been installed:

```bash
sudo apt install build-essential pkg-config libssl-dev
```
