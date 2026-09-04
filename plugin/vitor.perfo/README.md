# Perfo Omarchy Plugin

This plugin adds a live CPU, memory, I/O, network, and filesystem widget to
the Omarchy bar. The popup starts on an aggregate dashboard and uses the left
and right controls to move through focused pages.

## Runtime Dependency

The plugin runs `perfo stream --json`. Install the release binary in
`~/.local/bin/perfo`, or set `PERFO_BIN` to an executable path before starting
the Omarchy shell.

## Local Installation

```bash
cargo install --path . --root "$HOME/.local" --force
mkdir -p "$HOME/.config/omarchy/plugins/vitor.perfo"
cp plugin/vitor.perfo/* "$HOME/.config/omarchy/plugins/vitor.perfo/"
omarchy plugin validate "$HOME/.config/omarchy/plugins/vitor.perfo"
omarchy plugin enable vitor.perfo --section right
omarchy restart shell
```
