"""CLI for draft files. It never starts or imports a user script."""
from __future__ import annotations
import argparse
import json
from pathlib import Path
import sys
import time

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
    run = commands.add_parser('run', help='explicitly run a saved Python revision in the foreground')
    run.add_argument('id'); run.add_argument('--revision',required=True)
    args = parser.parse_args(argv)
    try:
        workspace = ScriptWorkspace(args.root)
        if args.command == 'run':
            from .script_runner import ScriptRunner
            with ScriptRunner(workspace) as runner:
                receipt = runner.run(args.id,args.revision)
                print(json.dumps(receipt,ensure_ascii=False),flush=True)
                try:
                    while receipt['state'] in {'running','stopping'}:
                        time.sleep(.1); receipt = runner.status(receipt['id'])
                except KeyboardInterrupt:
                    receipt = runner.stop(receipt['id'])
                    print(json.dumps(receipt,ensure_ascii=False)); return 130
                print(json.dumps(receipt,ensure_ascii=False))
                return 0 if receipt['exit_code'] == 0 else 1
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
