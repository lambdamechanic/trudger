# Beads (beads_rust) - AI-Native Issue Tracking

Welcome to beads_rust! This repository uses **br** (beads_rust) for issue tracking - a modern, AI-native tool designed to live directly in your codebase alongside your code.

**Note:** `br` is non-invasive and never executes git commands. After `br sync --flush-only`, you must manually run `git add .beads/ && git commit`.

## What is beads_rust?

beads_rust is issue tracking that lives in your repo, making it perfect for AI coding agents and developers who want their issues close to their code. No web UI required - everything works through the CLI and integrates seamlessly with git.

**Learn more:** [github.com/Dicklesworthstone/beads_rust](https://github.com/Dicklesworthstone/beads_rust)

## Quick Start

### Essential Commands

```bash
# Create new issues
br create "Add user authentication"

# View all issues
br list

# View issue details
br show <issue-id>

# Update issue status
br update <issue-id> --status in_progress
br update <issue-id> --status done

# Sync with git remote
br sync --flush-only
git add .beads/
git commit -m "sync beads"
```

### Working with Issues

Issues in beads_rust are:
- **Git-native**: Stored in `.beads/issues.jsonl` and synced like code
- **AI-friendly**: CLI-first design works perfectly with AI coding agents
- **Branch-aware**: Issues can follow your branch workflow
- **Always in sync**: Run `br sync --flush-only` and commit `.beads/` with your changes

## Why beads_rust?

✨ **AI-Native Design**
- Built specifically for AI-assisted development workflows
- CLI-first interface works seamlessly with AI coding agents
- No context switching to web UIs

🚀 **Developer Focused**
- Issues live in your repo, right next to your code
- Works offline, syncs when you push
- Fast, lightweight, and stays out of your way

🔧 **Git Integration**
- Manual sync after `br sync --flush-only`
- Branch-aware issue tracking
- Intelligent JSONL merge resolution

## Get Started with beads_rust

Try beads_rust in your own projects:

```bash
# Install beads_rust
# Follow the install instructions in the beads_rust repo.

# Initialize in your repo
br init

# Create your first issue
br create "Try out beads_rust"
```

## Learn More

- **Documentation**: See [github.com/Dicklesworthstone/beads_rust](https://github.com/Dicklesworthstone/beads_rust)
- **Quick Start Guide**: Run `br quickstart`
- **Examples**: See [github.com/Dicklesworthstone/beads_rust](https://github.com/Dicklesworthstone/beads_rust)

---

*beads_rust: Issue tracking that moves at the speed of thought* ⚡
