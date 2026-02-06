## 1. Implementation
- [x] 1.1 Update Rust CLI arg parsing to support subcommands (at least `doctor`) and remove positional task ids.
- [x] 1.2 Implement `-t/--task` parsing for manual task ids, supporting repeated `-t` and comma-separated values; preserve ordering.
- [x] 1.3 Ensure `-t/--task` is rejected in doctor mode with a clear error.
- [x] 1.4 Update usage/help text and README examples.
- [x] 1.5 Add tests for CLI parsing (including migration error for positional task ids).
