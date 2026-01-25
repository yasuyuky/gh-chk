# GitHub CLI Extension - gh chk

This extension allows you to interact with various GitHub features, such as pull requests, issues, contributions, notifications, and track assignees, all within your command line.

## Installation

```
gh extension install yasuyuky/gh-chk
```

## Usage

```
gh chk [OPTIONS] <COMMAND>
```

## Commands

- `prs` - Show pull requests of the repository or user.
- `tui` - Interactive TUI for pull requests.
- `issues` - Show issues of the repository or user.
- `contributions` - Show contributions of the user.
- `notifications` - Show notifications of the user.
- `track-assignees` - Track assignees of the issues or pull requests.
- `search` - Search repositories.
- `login` - Login to GitHub.
- `logout` - Logout from GitHub.
- `help` - Print this message or the help of the given subcommand(s).

## Options

- `-f <FORMAT>` - Set output format. Default: `text`. Possible values: `text`, `json`.
- `-h, --help` - Print help.

For more usage information, you can run `gh-chk help <COMMAND>` to get details on how to use each command.

## Authentication

`gh-chk` reads tokens from `~/.config/gh/hosts.yml`, falling back to `~/.config/gh-chk/config.toml`, then the `GITHUB_TOKEN` environment variable if needed.

## Examples

```
gh chk prs owner/repo
gh chk -f json issues owner/repo
gh chk track-assignees owner/repo 10
gh chk search "language:rust stars:>1000"
```

## TUI

```
gh chk tui [<owner>[/<repo>]]
```

Basic keys:
- `q` - Quit.
- Arrow keys or `j`/`k` - Move selection.
- `Enter` or `o` - Open the selected pull request in browser.
