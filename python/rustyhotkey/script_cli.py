"""CLI for draft files. It never starts or imports a user script."""
from __future__ import annotations
import argparse
import json
from pathlib import Path
import sys

from .scripts import ScriptWorkspace


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog='rhk scripts')
    parser.add_argument('--root', type=Path, help='caller-owned draft directory')
    commands = parser.add_subparsers(dest='command', required=True)
    commands.add_parser('list')
    for command in ('read', 'save'):
        sub = commands.add_parser(command)
        sub.add_argument('id'); sub.add_argument('extension', choices=('py','lua','json'))
        if command == 'save':
            sub.add_argument('--file', type=Path, required=True, help='UTF-8 text file to save as a draft')
            sub.add_argument('--revision', help='current revision for updating an existing draft')
    args = parser.parse_args(argv)
    try:
        workspace = ScriptWorkspace(args.root)
        if args.command == 'list': result = {'scripts':workspace.list_scripts()}
        elif args.command == 'read': result = workspace.read_script(args.id,args.extension)
        else:
            with args.file.open('rb') as source: raw = source.read(workspace.max_file_bytes + 1)
            if len(raw) > workspace.max_file_bytes: raise ValueError('source exceeds the draft size limit')
            result = workspace.save_script(args.id,args.extension,raw.decode('utf-8'),args.revision)
        print(json.dumps(result,ensure_ascii=False)); return 0
    except (OSError,ValueError) as error:
        print(json.dumps({'error':str(error)},ensure_ascii=False),file=sys.stderr); return 1


if __name__ == '__main__': raise SystemExit(main())
