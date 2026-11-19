"""Record CPython's results for the parity fixtures in this folder.

Usage: python -I tests/fixtures/py_parity.py <group>... > tests/fixtures/<fixture>.json

Each group is a key of the fixture: json, json_pairs, urlsplit, urlunsplit,
urljoin, unquote, quote, parse_qsl and iso for py_formats.json; repr,
splitlines, int, float and strip for py_text.json. The fixture's `generator`
field names the Python version and the command that wrote it.
"""
import datetime
import json
import platform
import sys
import urllib.parse as up

I128_MAX = 2**127 - 1


def err(e):
    return {"error": str(e)}


def loads_case(text):
    try:
        value = json.loads(text)
    except ValueError as e:
        return {"in": text, **err(e)}
    default = json.dumps(value, ensure_ascii=False)
    if any(0xD800 <= ord(c) <= 0xDFFF for c in default):
        # Python keeps a lone surrogate; a Rust string cannot hold one, so the
        # port refuses the document. This row records that difference.
        return {"in": text, "differs_from_cpython": "CPython keeps the lone surrogate; the port refuses it"}
    return {
        "in": text,
        "default": default,
        "compact": json.dumps(value, ensure_ascii=False, separators=(",", ":")),
        "indent": json.dumps(value, ensure_ascii=False, indent=2),
        "truthy": bool(value),
    }


JSON = [
    "null", "true", "false", "0", "-0", "1e2", "-1.5E-3", "1.0", "12345678901234567890123456789012345678901234567890",
    "-12345678901234567890123456789012345678901234567890", "NaN", "Infinity", "-Infinity", "1e400", "-1e400",
    '""', '"a\\"b\\\\c\\/d\\be\\ff\\ng\\rh\\ti"', '"\\u00e9\\u4e2d"', '"\\ud83d\\ude00"', '"\\ud83d\\u0041"',
    '"\\ud83dx"', '"\\uZZZZ"', '"\\u12"', '"\\ud83d\\uZZZZ"', '"\\x"', '"a\nb"', '"\x1f"', '"\u007f\u0085"',
    "[]", "{}", "[1, [2, [3]]]", '{"a": 1, "b": [true, null], "c": {"d": "e"}}', '{"a": 1, "a": 2}',
    "[1,]", "[1 2]", "{,}", '{"a" 1}', '{"a": 1,}', "{1: 2}", '{"a": 1 "b": 2}', "", " ", "[", "{", '"abc',
    "tru", "nul", "-", "01", "1.", "1e", ".5", "1 2", "[] x", "\ufeff[]", "  [ 1 , 2 ]  ", '"\t"',
    "1" * 4301, "-" + "1" * 4300, "0." + "1" * 5000, '{"k": [0.1, 1e-7, 1e16, 123456789.0]}',
]

PAIRS = [
    ("1", "1.0"), ("0", "-0.0"), ("0", "false"), ("1", "true"), ("2", "true"), ("1.5", "1.5"),
    ("12345678901234567890", "12345678901234567890.0"), ("12345678901234567891", "12345678901234567890.0"),
    ("-12345678901234567890", "12345678901234567890"), ("[1, 2]", "[1.0, 2]"), ("[1]", "[1, 2]"),
    ('{"a": 1}', '{"a": 1.0}'), ('{"a": 1}', '{"b": 1}'), ('"1"', "1"), ("null", "null"), ("null", "0"),
    ("1e400", "1e400"), ("NaN", "NaN"), ("-1", "1"), ("0.0", "false"),
]


def split_case(url):
    try:
        s = up.urlsplit(url, missing_as_none=True)
    except ValueError as e:
        return {"in": url, **err(e)}
    out = {"in": url, "parts": list(s[:5]), "hostname": s.hostname, "username": s.username, "password": s.password}
    try:
        out["port"] = s.port
    except ValueError as e:
        out["port_error"] = str(e)
    return out


URLS = [
    "https://u:p@Host.Example:443/p;q?a=1#f", "https://[::1]:8080/x", "https://[v1.fe]/", "https://[v1.]/",
    "https://[vz.x]/", "https://[1.2.3.4]/", "https://[::1/", "https://::1]/", "https://[::1]x/",
    "https://host:/", "https://host:99999/", "https://host:-1/", "https://host:1a/", "http://h:0/",
    "//netloc/path", "/path?q#f", "path", "?q", "#f", "", "mailto:a@b", "file:///etc/x", "HTTPS://H/",
    "https://h/p?#", "https://h#", "https://h?", " \t https://h/\n\tx", "https://user@/", "https://@h/",
    "https://h%41/", "https://[fe80::1%25eth0]/", "https://[::ffff:1.2.3.4]/", "a1+.-:rest", "1a:rest",
    "https://h/\u00e9", "https://\u00e9.com/", "https://h:\u0661\u0662/",
]

UNSPLIT = [
    ["https", "h", "/p", "q", "f"], ["", "", "p", "", ""], ["", "", "//p", "", ""], ["https", "", "p", "", ""],
    ["https", "", "", "", ""], ["file", "", "/x", "", ""], ["", "h", "p", "", ""], ["mailto", "", "a@b", "", ""],
    ["https", "h", "", "", "f"], ["", "", "", "q", "f"], ["svn+ssh", "", "x", "", ""],
]

JOINS = [
    ["https://a/b/c/d;p?q", u] for u in [
        "g:h", "g", "./g", "g/", "/g", "//g", "?y", "g?y", "#s", "g#s", "g?y#s", ";x", "g;x", "",
        ".", "./", "..", "../", "../g", "../..", "../../", "../../g", "../../../g", "/./g", "/../g",
        "g.", ".g", "g..", "..g", "./../g", "./g/.", "g/./h", "g/../h", "http:g", "https:g", "https:",
        "https:?z", "https:#z", "//[::1/x",
    ]
] + [["", "x"], ["https://a/b", ""], ["mailto:a", "b"], ["https://a/b?q#f", ""], ["https://a", "b"],
     ["https://a/b/", "../../../x"], ["file:///a/b", "c"], ["https://a/b", "//c"], ["foo:bar", "baz"]]

UNQUOTE = ["a%20b", "%41%4a%4A", "%", "%4", "%zz", "%e2%82%ac", "%e2%82", "%ff", "a+b", "%C3%A9%", "plain", ""]
QUOTE = [["a b/c?d", "/"], ["\u00e9\u20ac~", ""], ["!*'();:@&=+$,/?#[]", ""], ["abc-._~", ""], ["%41", "%"]]
QSL = ["a=1&b=2", "a=1;b=2", "a&b=", "=x", "a=%20+b", "a=1&&b=2", "", "a=1&a=2", "%zz=%41"]


def iso_case(text):
    # `date` is toordinal(); `datetime` is microseconds from the start of
    # ordinal day 0 in local wall time (toordinal() days plus the time), then
    # the UTC offset in microseconds or null. These match the port's units.
    out = {"in": text}
    try:
        out["date"] = datetime.date.fromisoformat(text).toordinal()
    except ValueError:
        out["date"] = None
    try:
        dt = datetime.datetime.fromisoformat(text)
        # Microseconds from the start of ordinal day 0, so whole days are toordinal().
        local = (dt.replace(tzinfo=None) - datetime.datetime(1, 1, 1)) // datetime.timedelta(microseconds=1) + 86_400_000_000
        offset = dt.utcoffset()
        out["datetime"] = [local, None if offset is None else offset // datetime.timedelta(microseconds=1)]
    except ValueError:
        out["datetime"] = None
    return out


ISO = [
    "2026-11-19", "20261119", "2026-W47", "2026-W47-4", "2026W474", "2026-323", "2026323", "2024-02-29",
    "2026-02-29", "0000-01-01", "9999-12-31", "2026-13-01", "2026-1-01", "2026-11-19T10:00", "2026-11-19T10",
    "2026-11-19T10:00:00.5", "2026-11-19T10:00:00,123456789", "2026-11-19T10:00:00Z", "2026-11-19T10:00:00+10:00",
    "2026-11-19T10:00:00-0530", "2026-11-19T10:00:00+10:00:30.5", "2026-11-19 10:00:00", "2026-11-19T24:00:00",
    "2026-11-19T24:00:01", "2026-11-19T1000", "2026-11-19T100000", "2026-11-19T10:00:00+25:00",
    "2026-11-19T10:00:00.", "2026-11-19T10:00:00+10", "20261119T100000Z", "2026-W53", "2020-W53-7",
    "2026-W00", "2026-W01-0", "2026-W01-8", "2026-000", "2026-366", "2024-366", "2026-11-19X10:00",
    "2026-11-19T10:00:00.1234567", "2026-11-19T10:00:00+10:00:00.123456", "2026-11-19T10:00:60",
    "2026-11-19T\u0661\u0660:00", "2026-11-19T10:00:00 ", "2026-11-19T10:00:00+1000",
]

STRINGS = [
    "", "plain", "it's", 'say "hi"', "both ' and \"", "tab\there", "cr\rlf\n", "back\\slash", "\x00\x1f\x7f",
    "\x80\x9f\xa0\xad", "\u00e9\u4e2d", "\U0001f600", "\u200b\u2028\ufeff", "\U000e0001", "\U0010ffff", "\ud7ff",
]
LINES = ["", "a", "a\n", "a\nb", "a\r\nb", "a\rb", "a\r", "a\r\n", "\n\n", "a\x0bb\x0cc\x1cd\x1de\x1ef\x85g\u2028h\u2029i"]
INTS = ["0", "-0", "+5", " 42 ", "1_000", "_1", "1__0", "1_", "١٢", " 3　", "12a", "", "-",
        "+", "0x10", "9" * 50, "-" + "9" * 50, "1" * 4301, "0" * 5000 + "1", "0" * 4300 + "1", "\t7\n",
        "\x0b-8\x0c\r", "\x1c9\x1f", "\x857 ", "1 2"]
FLOATS = ["1.5", "-0.0", "inf", "-Infinity", "nan", "+NaN", "1e308", "1e309", "-1e309", "1_0.5", "1e-400",
          "0.1", "١.5", "  2.5  ", "\t2.5\n", "\x0b-1e3\r", "\x1cinf\x1f", "1.5.5", "", "e5", ".5", "5.",
          "1e", "1_e5", "0x1p3", "1E+2", "123456789012345678", "1e16", "1e-5", "0.0001", "5e-324"]
STRIPS = [" a ", "\t\n a \x0b\x0c\r", "\u3000a\u2003", "\x1c\x1d\x1e\x1fa\x85", "\u200ba\u200b"]


def main():
    groups = sys.argv[1:]
    out = {
        "generator": f"Python {platform.python_version()}: py_parity.py {' '.join(groups)}",
        "json": [loads_case(t) for t in JSON],
        "json_pairs": [[a, b, json.loads(a) == json.loads(b)] for a, b in PAIRS],
        "urlsplit": [split_case(u) for u in URLS],
        "urlunsplit": [[p, up.urlunsplit(p)] for p in UNSPLIT],
        "urljoin": [],
        "unquote": [],
        "quote": [[s, safe, up.quote(s, safe=safe)] for s, safe in QUOTE],
        "parse_qsl": [[q, [list(p) for p in up.parse_qsl(q)]] for q in QSL],
        "iso": [iso_case(t) for t in ISO],
        "repr": [[s, repr(s)] for s in STRINGS],
        "splitlines": [[s, s.splitlines()] for s in LINES],
        "int": [],
        "float": [],
        "strip": [[s, s.strip(), s.lstrip(), s.rstrip()] for s in STRIPS],
    }
    for base, url in JOINS:
        try:
            out["urljoin"].append([base, url, up.urljoin(base, url)])
        except ValueError as e:
            out["urljoin"].append([base, url, None, str(e)])
    for s in UNQUOTE:
        try:
            strict = up.unquote(s, errors="strict")
        except UnicodeDecodeError:
            strict = None
        out["unquote"].append([s, up.unquote(s), strict])
    for t in INTS:
        try:
            v = int(t)
            out["int"].append([t, str(max(-I128_MAX, min(I128_MAX, v)))])
        except ValueError:
            out["int"].append([t, None])
    for t in FLOATS:
        try:
            out["float"].append([t, repr(float(t))])
        except ValueError:
            out["float"].append([t, None])
    unknown = set(groups) - set(out) | ({"generator"} & set(groups))
    if not groups or unknown:
        sys.exit(f"name one or more groups; unknown: {sorted(unknown)}")
    sections = []
    for key, value in out.items():
        if key != "generator" and key not in groups:
            continue
        if isinstance(value, list):
            rows = ",\n".join(json.dumps(v, ensure_ascii=True) for v in value)
            sections.append(f'"{key}": [\n{rows}\n]')
        else:
            sections.append(f'"{key}": {json.dumps(value)}')
    sys.stdout.write("{\n" + ",\n".join(sections) + "\n}\n")


main()
