"""Minimal reader for Beyond All Reason unit definition files.

BAR unit files are data-only Lua: `return { name = { key = value, ... } }` with
strings, numbers, booleans, nested tables and `[n] = ` indices. This parses just
that subset, which is enough to derive simulator catalog entries without a game.
"""

import re

_TOKEN = re.compile(
    r"""\s*(?:
        (?P<comment>--\[\[.*?\]\]|--[^\n]*)
      | (?P<longstring>\[(?P<eq>=*)\[.*?\](?P=eq)\])
      | (?P<string>"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*')
      | (?P<number>-?(?:0[xX][0-9a-fA-F]+|\d+\.?\d*(?:[eE][-+]?\d+)?|\.\d+))
      | (?P<name>[A-Za-z_][A-Za-z_0-9]*)
      | (?P<punct>[{}\[\]=,;./*])
    )""",
    re.X | re.S,
)


def _tokens(text):
    pos = 0
    out = []
    while pos < len(text):
        m = _TOKEN.match(text, pos)
        if not m or m.end() == pos:
            if text[pos:].strip() == "":
                break
            raise ValueError("lua parse error near %r" % text[pos : pos + 40])
        pos = m.end()
        kind = "longstring" if m.group("longstring") is not None else m.lastgroup
        if kind == "comment":
            continue
        out.append((kind, m.group(kind)))
    return out


class _Parser:
    def __init__(self, tokens):
        self.t = tokens
        self.i = 0

    def peek(self):
        return self.t[self.i] if self.i < len(self.t) else (None, None)

    def take(self):
        tok = self.t[self.i]
        self.i += 1
        return tok

    def value(self):
        result = self.atom()
        # Unit files occasionally write `1 / 70`; only * and / on numbers are supported.
        while isinstance(result, float) and self.peek() in (("punct", "/"), ("punct", "*")):
            op = self.take()[1]
            rhs = self.atom()
            if not isinstance(rhs, float):
                raise ValueError("arithmetic on non-number")
            result = result / rhs if op == "/" else result * rhs
        return result

    def atom(self):
        kind, text = self.take()
        if kind == "longstring":
            level = text.index("[", 1)  # number of '=' plus the first '['
            inner = text[level + 1 : len(text) - level - 1]
            return inner[1:] if inner.startswith("\n") else inner
        if kind == "string":
            return bytes(text[1:-1], "utf-8").decode("unicode_escape")
        if kind == "number":
            return float(int(text, 16)) if text.lower().startswith("0x") else float(text)
        if kind == "name":
            if text == "true":
                return True
            if text == "false":
                return False
            if text == "nil":
                return None
            return text
        if (kind, text) == ("punct", "{"):
            return self.table()
        raise ValueError("unexpected token %r" % (text,))

    def table(self):
        out = {}
        index = 1
        while True:
            kind, text = self.peek()
            if (kind, text) == ("punct", "}"):
                self.take()
                return out
            if (kind, text) in (("punct", ","), ("punct", ";")):
                self.take()
                continue
            if (kind, text) == ("punct", "["):
                self.take()
                key = self.value()
                self.take()  # ]
                self.take()  # =
                out[key] = self.value()
                continue
            if kind == "name" and self.t[self.i + 1] == ("punct", "="):
                self.take()
                self.take()
                out[text.lower()] = self.value()
                continue
            out[index] = self.value()
            index += 1


def parse_unit_file(path):
    """Return {unit_name: table} with every string key lower-cased."""
    with open(path, encoding="utf-8") as handle:
        tokens = _tokens(handle.read())
    if tokens[0] != ("name", "return"):
        raise ValueError("%s does not start with return" % path)
    parser = _Parser(tokens[1:])
    parser.take()  # {
    return parser.table()
