#!/usr/bin/env python3
"""Check Git's staged content for material that does not belong in a public repo."""
import argparse
import hashlib
import ipaddress
import os
from pathlib import Path
import re
import subprocess
import sys

# Hashes of case-insensitive, punctuation-free private identifiers. These are
# deny rules, not credentials. Additional local hashes may only extend the set.
DENIED = {
    '9e1adfdb3b0d1064aefc09d6f7118daa76ab1eca1352adad4d4494c1dac96c40',
    '86d16c29bc44e6ec89def7f536cb10caba3f8c6656f7abe5144cb6fe36f93939',
    '93374eb2a8799f8254ae5136cd3996bb6aea260cdfae2e962da2e8e81eea2d9f',
    '0942555cb3a29686b0f84bfb83d85c0249c98a08fee4176775712cf51b785a4f',
    'e5336673bc2fb58f455589eea41e7a634d4d27f88d9af4f3eb85d66bf6fb7398',
    '35838647ddd3303c35b39440f80c139731d84c46f5094d2541c9076cae86263a',
    'b0a63a560d1afb7b9fb0b4a59a2479e42fdbe6c76b74fc61d3ce4f437f0486a6',
}
APPROVED_BINARY = {'python/rhk/ui_assets/portraits.png': 'daca2cd36820970ba2f8c45e26276bccd4e260e9037123809bd95142246874fc'}
BLOCKED_PARTS = {'profiles', 'recordings', 'runtime', 'sessions', 'session', 'cache', '.cache', '__pycache__'}
BLOCKED_SUFFIXES = {'.db', '.sqlite', '.sqlite3', '.log', '.dmp', '.dump', '.pyc', '.pfx', '.p12', '.key'}
PATH_PATTERNS = (
    re.compile(r'(?i)(?<![a-z0-9])(?:[a-z]:[\\/](?:[^\s\"\'<>|:]+[\\/]?)+)'),
    re.compile(r'(?i)(?<![a-z0-9])/(?:home|users)/[a-z0-9._-]+(?:/[^\s\"\']*)?'),
    re.compile(r'(?<![\\])\\\\[a-zA-Z0-9][a-zA-Z0-9_.-]*\\[a-zA-Z0-9_$.-]+'),
    re.compile(r'(?i)\b(?:spark|dgx)[-_][a-z0-9]{3,}(?:[-_][a-z0-9]+)*\b'),
)
IP = re.compile(r'(?<![\d.])(?:\d{1,3}\.){3}\d{1,3}(?![\d.])')
TOKEN = re.compile(r'[a-z0-9]+', re.I)
PRIVATE_KEY = re.compile(r'-----BEGIN (?:RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----')
SECRET = re.compile(r'''(?ix)\b(?:api[_-]?key|access[_-]?token|secret[_-]?key|password|client[_-]?secret)\b\s*[:=]\s*["']([a-z0-9_./+=-]{12,})["']''')
SECRET_TOKEN = re.compile(r'\b(?:sk-[A-Za-z0-9_-]{20,}|gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,})\b')


def git(*args):
    return subprocess.run(['git', *args], check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE).stdout


def denied_hashes():
    denied = set(DENIED)
    local = os.environ.get('RHK_PRIVATE_MARKER_HASHES_FILE')
    if local:
        for line in Path(local).read_text(encoding='ascii').splitlines():
            line = line.strip().lower()
            if not line or line.startswith('#'):
                continue
            if not re.fullmatch(r'[0-9a-f]{64}', line):
                raise ValueError('local marker file must contain one SHA-256 hash per line')
            denied.add(line)
    return denied


def has_marker(text, denied):
    tokens = TOKEN.findall(text.lower())
    for i in range(len(tokens)):
        joined = ''
        for token in tokens[i:i + 4]:
            joined += token
            if hashlib.sha256(joined.encode('ascii')).hexdigest() in denied:
                return True
    return False


def inspect(path, content, denied):
    parts = path.lower().replace('\\', '/').split('/')
    name = parts[-1]
    if any(part in BLOCKED_PARTS for part in parts) or name == '.env' or name.startswith('.env.') or Path(name).suffix in BLOCKED_SUFFIXES:
        return 'private runtime, credential, or cache path'
    if has_marker(path, denied):
        return 'private identifier in path'
    digest = hashlib.sha256(content).hexdigest()
    if path in APPROVED_BINARY:
        return None if digest == APPROVED_BINARY[path] else 'approved asset differs from its reviewed digest'
    try:
        text = content.decode('utf-8')
    except UnicodeDecodeError:
        return 'unreviewed binary content'
    if '\0' in text:
        return 'unreviewed binary content'
    if has_marker(text, denied):
        return 'private identifier in content'
    if any(pattern.search(text) for pattern in PATH_PATTERNS):
        return 'machine-specific path or host'
    for match in IP.finditer(text):
        try:
            ip = ipaddress.IPv4Address(match.group())
            if any(ip in ipaddress.IPv4Network((base, bits)) for base, bits in ((0x0a000000, 8), (0xac100000, 12), (0xc0a80000, 16))):
                return 'private network address'
        except ValueError:
            pass
    if PRIVATE_KEY.search(text) or SECRET.search(text) or SECRET_TOKEN.search(text):
        return 'possible credential'
    return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument('--staged', action='store_true', help='scan added or modified staged files (default)')
    mode.add_argument('--all', action='store_true', help='scan all tracked index entries')
    args = parser.parse_args()
    try:
        denied = denied_hashes()
        raw = git('ls-files', '-z', '--cached') if args.all else git('diff', '--cached', '--name-only', '-z', '--diff-filter=ACMR', '--no-renames')
        paths = [os.fsdecode(p) for p in raw.split(b'\0') if p]
        failures = []
        for path in paths:
            # Reading :path uses the index, including for --all; the worktree is irrelevant.
            reason = inspect(path, git('show', ':' + path), denied)
            if reason:
                failures.append((path, reason))
        for path, reason in failures:
            print(f'public guard: {path!r}: {reason}', file=sys.stderr)
        if failures:
            print('Remove private content from the index before committing.', file=sys.stderr)
            return 1
        print(f'public guard: {len(paths)} indexed files checked')
        return 0
    except (OSError, ValueError, subprocess.CalledProcessError) as exc:
        print(f'public guard could not complete: {exc}', file=sys.stderr)
        return 2


if __name__ == '__main__':
    sys.exit(main())
