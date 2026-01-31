## 1. Implementation
- [ ] 1.1 Update trudger spec deltas for logging, parsing, prompt substitution, and reexec behavior
- [ ] 1.2 Implement YAML parse failure handling and clear error messaging
- [ ] 1.3 Ensure prompt substitution preserves special characters
- [ ] 1.4 Standardize quit/error logging with control-character escaping
- [ ] 1.5 Ensure error trap exits via quit path
- [ ] 1.6 Use resolved reexec path when available
- [ ] 1.7 Add tests for parse failure, prompt substitution, and logging/error behavior
- [ ] 1.8 Run validation: `openspec validate update-trudger-logging-and-parsing --strict --no-interactive`
