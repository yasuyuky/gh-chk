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
- `issues` - Show issues of the repository or user.
- `contributions` - Show contributions of the user.
- `notifications` - Show notifications of the user.
- `track-assignees` - Track assignees of the issues or pull requests.
- `login` - Login to GitHub.
- `logout` - Logout from GitHub.
- `help` - Print this message or the help of the given subcommand(s).

## Options

- `-f <FORMAT>` - Set output format. Default: `text`. Possible values: `text`, `json`.
- `-h, --help` - Print help.

For more usage information, you can run `gh-chk help <COMMAND>` to get details on how to use each command.
