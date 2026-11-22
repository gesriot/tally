"""Проверка двух сценариев на собранных tally и tally-ui в одной папке.

Запуск: python scripts/check-quick-add-startup.py ПУТЬ_К_TALLY
Используются временная SQLite-база и отдельный каталог обмена. Окно не требуется.
"""

import argparse
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import time
from contextlib import closing, contextmanager, ExitStack


VERSION = 31415
TIMEOUT_TEXT = "не подтвердил открытие"


def setup_case(tally, root):
    db = root / "future.db"
    with closing(sqlite3.connect(db)) as connection:
        connection.execute("PRAGMA journal_mode = WAL")
        connection.execute(f"PRAGMA user_version = {VERSION}")
    ipc = root / "ipc"
    env = dict(os.environ, TALLY_IPC_DIR=str(ipc))
    env.pop("TALLY_TODAY", None)
    return [str(tally), "--db", str(db), "quick-add"], env, db, ipc


@contextmanager
def client(command, env):
    process = subprocess.Popen(
        command, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
        text=True, encoding="utf-8", errors="replace",
    )
    try:
        yield process
    finally:
        if process.poll() is None:
            process.kill()
        process.communicate()


def require_version_error(process, description):
    _, error = process.communicate(timeout=20)
    if process.returncode != 1 or f"v{VERSION}" not in error:
        raise AssertionError(f"{description}: код {process.returncode}, stderr={error!r}")


def two_clients_on_startup_error(tally, root):
    command, env, db, ipc = setup_case(tally, root)
    with closing(sqlite3.connect(db)) as writer, ExitStack() as stack:
        # Tally занимает instance.lock, затем ждёт BEGIN IMMEDIATE в миграции.
        writer.execute("BEGIN IMMEDIATE")
        clients = [stack.enter_context(client(command, env)) for _ in range(2)]
        # На текущем протоколе дожидаемся передачи второго запроса. Также допускаем
        # реализацию, в которой клиенты ждут готовности окна до отправки запроса.
        end = time.monotonic() + 2.5  # busy_timeout хранилища равен 5 с.
        while not any(ipc.glob("*/request-*")) and time.monotonic() < end:
            time.sleep(0.01)
        writer.rollback()
        errors = []
        for i, process in enumerate(clients, start=1):
            try:
                require_version_error(process, f"клиент {i}")
            except (AssertionError, subprocess.TimeoutExpired) as error:
                errors.append(str(error))
        if errors:
            raise AssertionError("\n".join(errors))


@contextmanager
def hold_gate(path):
    with path.open("r+b") as file:
        if os.name == "nt":
            import msvcrt
            msvcrt.locking(file.fileno(), msvcrt.LK_NBLCK, 1)
        else:
            import fcntl
            fcntl.flock(file, fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield  # Закрытие файла освобождает блокировку на обеих платформах.


def timeout_while_gate_is_busy(tally, root):
    command, env, _, ipc = setup_case(tally, root)
    with client(command, env) as probe:
        require_version_error(probe, "контрольный запуск")
    gate, = ipc.glob("*/requests.lock")
    with hold_gate(gate), client(command, env) as process:
        # Задокументированы 15 секунд. Ещё 4 отведены на старт и планировщик.
        try:
            _, error = process.communicate(timeout=19)
        except subprocess.TimeoutExpired as timeout:
            raise AssertionError("CLI всё ещё ждёт requests.lock через 19 с") from timeout
        if process.returncode != 1 or TIMEOUT_TEXT not in error:
            raise AssertionError(f"ожидался тайм-аут, код {process.returncode}, stderr={error!r}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("tally", type=Path, help="путь к tally или tally.exe")
    tally = parser.parse_args().tally.resolve()
    if not tally.is_file() or not tally.with_name("tally-ui" + tally.suffix).is_file():
        parser.error("укажите собранный tally, рядом с которым находится tally-ui")
    failures = 0
    for check in (two_clients_on_startup_error, timeout_while_gate_is_busy):
        with tempfile.TemporaryDirectory(prefix="tally-startup-review-") as tmp:
            try:
                check(tally, Path(tmp))
            except (AssertionError, OSError, ValueError, subprocess.TimeoutExpired) as error:
                print(f"FAIL {check.__name__}: {error}", file=sys.stderr)
                failures += 1
            else:
                print(f"PASS {check.__name__}")
    return int(bool(failures))


if __name__ == "__main__":
    sys.exit(main())
