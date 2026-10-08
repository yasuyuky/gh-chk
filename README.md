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
- `search` - Search code.
- `login` - Login to GitHub.
- `logout` - Logout from GitHub.
- `help` - Print this message or the help of the given subcommand(s).

## Options

- `-f <FORMAT>` - Set output format. Default: `text`. Possible values: `text`, `json`.
- `-h, --help` - Print help.

For more usage information, you can run `gh-chk help <COMMAND>` to get details on how to use each command.

## Authentication

`gh-chk` reads tokens from `gh` auth (`~/.config/gh/hosts.yml` or `gh auth token`), falling back to `~/.config/gh-chk/config.toml`, then the `GITHUB_TOKEN` environment variable if needed.

## Examples

```
gh chk prs owner/repo
gh chk prs --merge owner/repo
gh chk -f json issues owner/repo
gh chk track-assignees owner/repo 10
gh chk search "language:rust repo:owner/repo"
```

## Merging pull requests

`gh chk prs --merge owner/repo` and the TUI's `m` key use GitHub's
[asynchronous merge API](https://docs.github.com/en/rest/pulls/pulls#merge-a-pull-request-asynchronously).
The TUI lets GitHub evaluate whether the selected PR can be merged or queued.
`prs --merge` retains the existing `CLEAN` state requirement and processes those
PRs sequentially in text mode; `-f json` continues to list PRs without merging them.

Merges follow the target branch's merge queue configuration and do not bypass
repository rules. For stacked PRs, GitHub also includes open downstack PRs.
The command polls every two seconds for up to 60 seconds after submission:

- `Merged PR` means the merge completed.
- `Added PR ... to the merge queue (not yet merged)` means queue admission completed.
- Failures report GitHub's reason. A polling timeout includes the request ID and
  status endpoint; the merge may still be running on GitHub.

The TUI remains usable while waiting. Quitting does not cancel a submitted merge.

## TUI

```
gh chk tui [<owner>[/<repo>]]
```

Basic keys:
- `q` - Quit.
- Arrow keys or `j`/`k` - Move selection.
- `Enter` or `o` - Open the selected pull request in browser.
- `p` - Open your GitHub profile in browser.
- `m` - Merge the selected pull request, or add it to its configured merge queue.
- `s` - Search code. In the search input, use `Up`/`Down` to browse search history.
