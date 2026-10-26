# Command history

Command recording is off by default. Enable **Settings → General → Command
history via shell integration** to record execution events from a remote shell.
Turning it off stops new recording; existing entries remain in the local history
database and can be removed with their session in the session dashboard.

EvertySSH never reconstructs commands from keyboard input. Without integration,
terminal use works normally and no commands are added to history. Passwords typed
into `sudo`, `ssh`, database clients and other interactive programs are not shell
execution events, so they are not recorded.

## Bash

Copy [shell-integration.bash](shell-integration.bash) to the remote server, for
example as `~/.config/evertyssh/shell-integration.bash`. Add this line to that
server's `~/.bashrc`:

```bash
source ~/.config/evertyssh/shell-integration.bash
```

Start a new interactive Bash session, or source the file in an existing session.
Bash 4.4 or later and shell history (`set -o history`) are required. The hook uses
`PS0`, preserves an existing `PS0`, and does not replace DEBUG traps or
`PROMPT_COMMAND`. Commands come from Bash history after readline editing,
completion, and history expansion. Compound and multiline commands are recorded
as accepted by the shell. History entries suppressed by `HISTCONTROL` are not
recorded again. Repeated identical commands suppressed by `ignoredups` are not
new entries either.

Shell commands can themselves contain credentials, such as `curl -H
'Authorization: Bearer …'`. Those command arguments are part of history. Use
environment variables or secret input instead. With `HISTCONTROL=ignorespace`,
prefix a command with a space to omit it from both Bash history and EvertySSH's
integration history.

## Other shells and protocol

EvertySSH accepts OSC 633 `E` (command line) followed by `C` (execution started),
using `\\` and `\xHH` escapes. An `E` without a subsequent `C` is not recorded.
Prompt marker `A` clears an unexecuted command. Commands are limited to 4096 UTF-8
bytes; blank commands, commands beginning with a space, and unexpected control
characters are omitted. Events in an alternate screen are ignored.

The bundled Bash hook adds an `everty-history:` identifier as the optional fourth
parameter of `E` and establishes its initial value using `P;EvertyHistoryId=…`.
This suppresses stale history entries when Bash ignores a command. Other
integrations may omit this identifier.

Only enable history for servers you trust. Shell integration events are sent by
the remote server, and are not independently authenticated command audit records.
Existing historical entries from earlier versions are not automatically removed.

## Русский

Запись выключена по умолчанию. Включите её в **Настройки → Параметры → История
команд через интеграцию с оболочкой**, скопируйте `shell-integration.bash` на сервер
и подключите через `source` в `~/.bashrc`. Требуется интерактивный Bash 4.4+ с
включённой историей. Ввод паролей в `sudo` и других программах не записывается.
Аргументы выполненных команд могут содержать секреты; отключайте запись для
таких сессий или используйте `HISTCONTROL=ignorespace` и пробел перед командой.
Ранее сохранённые записи остаются в базе; удалить их можно вместе с сессией.
