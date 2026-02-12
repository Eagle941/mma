Here are the challenges to think about during review and testing:

- Investigate how to pass and share order book updates to the strategy
- Investigate how to represent prices (f64 is the simplest, but creates
problems)
- Investigate solutions more efficient than `triple_buffer`
- Rewrite the bybit library
