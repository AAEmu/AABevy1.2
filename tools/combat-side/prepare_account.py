#!/usr/bin/env python3
"""Prepare one lane account for the combat recorder dry run.

The lane databases are throwaway.  This helper makes sure the login user
exists with the launcher password and inserts/updates the game database
``accounts`` row with GM access:

* ``access_level = 100`` (the task's dry-run requirement), inserted when the
  row is missing;
* the login ``users.password`` set to ``base64(sha256(password))``, the format
  the launcher password file expects.

It only touches the lane databases named by the lane's ``*.env`` file and
never prints the password.

Usage:
    python tools/combat-side/prepare_account.py \\
        --lane-env <lane>.env \\
        --user clienttest \\
        --password-file <password-file>
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import os
import subprocess
import sys
from pathlib import Path

# The mysql client on PATH unless AA_MYSQL names it. The port and user come from
# the lane's env file (MYSQL_PORT / MYSQL_USER) or the command line.
MYSQL = Path(os.environ.get("AA_MYSQL", "mysql"))


def read_lane(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8-sig").splitlines():
        line = line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        values[key.strip()] = value.strip()
    return values


MYSQL_PORT = "3306"
MYSQL_USER = "root"


def mysql(mysql_exe: Path, sql: str, database: str) -> list[list[str]]:
    command = [str(mysql_exe), "--host=127.0.0.1", f"--port={MYSQL_PORT}", f"--user={MYSQL_USER}",
               "--protocol=TCP", "--default-character-set=utf8mb4", "--batch", "--raw",
               "--skip-column-names", database, "-e", sql]
    result = subprocess.run(command, capture_output=True, text=True, encoding="utf-8",
                            timeout=60)
    if result.returncode != 0:
        raise SystemExit("mysql failed: " + result.stderr.strip())
    return [line.split("\t") for line in result.stdout.splitlines() if line]


def sql_string(value: str) -> str:
    return "'" + value.replace("\\", "\\\\").replace("'", "''") + "'"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--lane-env", type=Path, required=True)
    parser.add_argument("--user", default="clienttest")
    parser.add_argument("--password-file", type=Path, required=True)
    parser.add_argument("--access-level", type=int, default=100)
    parser.add_argument("--mysql", type=Path, default=MYSQL)
    parser.add_argument("--mysql-port", help="default: the lane's MYSQL_PORT, else 3306")
    parser.add_argument("--mysql-user", help="default: the lane's MYSQL_USER, else root")
    args = parser.parse_args()

    lane = read_lane(args.lane_env)
    global MYSQL_PORT, MYSQL_USER
    MYSQL_PORT = args.mysql_port or lane.get("MYSQL_PORT", MYSQL_PORT)
    MYSQL_USER = args.mysql_user or lane.get("MYSQL_USER", MYSQL_USER)
    login_db = lane.get("DB_LOGIN")
    game_db = lane.get("DB_GAME")
    if not login_db or not game_db:
        raise SystemExit(f"{args.lane_env} lacks DB_LOGIN/DB_GAME")

    password = args.password_file.read_text(encoding="utf-8-sig").strip()
    encoded = base64.b64encode(hashlib.sha256(password.encode()).digest()).decode()
    rows = mysql(args.mysql, f"SELECT id FROM users WHERE username={sql_string(args.user)} ORDER BY id LIMIT 1",
                 login_db)
    if rows:
        user_id = int(rows[0][0])
        mysql(args.mysql, f"UPDATE users SET password={sql_string(encoded)} WHERE id={user_id}", login_db)
    else:
        mysql(args.mysql, "INSERT INTO users (username,password,email,last_login,last_ip,created_at,"
              "updated_at,banned,ban_reason) VALUES "
              f"({sql_string(args.user)},{sql_string(encoded)},'',0,'',0,0,0,0)", login_db)
        user_id = int(mysql(args.mysql, f"SELECT id FROM users WHERE username={sql_string(args.user)} "
                            "ORDER BY id DESC LIMIT 1", login_db)[0][0])
    mysql(args.mysql, "INSERT INTO accounts (account_id, access_level) VALUES (%d, %d) "
          "ON DUPLICATE KEY UPDATE access_level=VALUES(access_level)"
          % (user_id, args.access_level), game_db)
    level = mysql(args.mysql, f"SELECT access_level FROM accounts WHERE account_id={user_id}", game_db)[0][0]
    print(f"account {args.user}: id={user_id}, access_level={level} ({game_db})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
