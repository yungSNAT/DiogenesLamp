"""Record BeautifulSoup's html.parser results for the html.rs parity fixture.

Usage: python -I tests/fixtures/html_parity.py > tests/fixtures/html_parity.json
with beautifulsoup4 installed. The fixture's `generator` field names the versions.
"""
import json
import platform
import sys

import bs4
from bs4 import BeautifulSoup

CASES = [
    # Text and whitespace.
    "<p> </p><p>\t</p><b>x</b>",
    "<div>a <span> </span> b</div>",
    # Character references in text.
    "<p>&notit; &notin; &no &amp &AMP; &ampx; &Amp;</p>",
    "<p>&#x; &#; &#x1F600; &#128512; &#xD800; &#55296; &#x80; &#150; &#13;</p>",
    "<p>&#99999999999999999999; &#x10FFFF; &#x110000; &#0;</p>",
    "<p>&#X41; &#x41 &#65 &#a; &#xg;</p>",
    "<p>x&</p>",
    "<p>x&#</p>",
    "<p>x&#x</p>",
    "<p>x&am</p>",
    "<p>x&#1</p>",
    "<p>x&# y</p>",
    "<p>x& y</p>",
    # Character references in attribute values.
    "<a title='&notit; &no &amp=1 &ampx &#65; &#x41; &#x; &#; &am'>t</a>",
    "<a title='&#99999999999; &#x110000; &#0; &#150; &copy=x &copyx &lt;'>t</a>",
    "<a title=\"a&#\">t</a><b title=\"&#x\">u</b><i title=\"&\">v</i>",
    # Attributes.
    "<a b=1 c = 2 d =\"3\" e= '4' f>t</a>",
    "<a title=x/y>t</a><i title=x/>u</i>",
    "<a title=\"x\"y=\"z\">t</a>",
    "<a ==x title=y>t</a>",
    "<a title='x' / >t</a>",
    "<a\ttitle\n=\n'x'>t</a>",
    "<a title=`x`>t</a>",
    "<a title=>t</a>",
    "<a title=\"unterminated>t</a>",
    "<a title='un\nterminated>t</a>",
    "<a/title=x>t</a>",
    "<a //title=x>t</a>",
    # Comments, declarations and processing instructions.
    "a<!---->b<!-- x -->c",
    "a<!-->b",
    "a<!--->b",
    "a<!-- x --!>b",
    "a<!-- x -- >b-->c",
    "a<!-- x --!b-->c",
    "a<!x>b<!>c",
    "a</ x>b</>c</1>d",
    "a<!DOCTYPE html>b<!doctype>c",
    "a<![CDATA[x]]>b<![if x]>c",
    "a<?php x ?>b",
    "a<!ELEMENT x>b",
    # Constructs cut off at the end of the input.
    "a<!-- unterminated",
    "a<!-- unterminated--",
    "a<!-- unterminated-",
    "a<!-- unterminated--!",
    "a<![CDATA[unterminated",
    "a<!DOCTYPE unterminated",
    "a<!x unterminated",
    "a<? unterminated",
    "a</unterminated",
    "a</",
    "a<",
    "a<b",
    "a<b c='x",
    "a < b",
    "a<1b",
    # Raw text and escapable raw text elements.
    "<script>a</b>c</script >d",
    "<script>a</scriptx>b</script>c",
    "<style>p{}</style><style>q</STYLE>x",
    "<script>unterminated",
    "<script>x</scr",
    "<textarea>a<b>&amp;</textarea>c",
    "<title>a &lt; <b></title>c",
    "<textarea>unterminated &amp;",
    "<xmp><b>x</b></xmp>y",
    "<noembed><b>x</noembed>y",
    "<iframe><b>x</iframe>y",
    "<plaintext><b>x",
    # Tree building.
    "<p>a<p>b</p></p>c",
    "<b><i>x</b>y</i>z",
    "</b>a<b>",
    "<br>a</br>b",
    "<img>a</img>b",
    "<a><a>x</a></a>",
    "<pre>\n  x\n</pre>",
    "<pre><b>  </b></pre>",
    "<ruby>a<rp>(</rp><rt>b</rt><rp>)</rp></ruby>",
    "<template><p>x</p></template>y",
    "<svg><foreignObject>x</foreignObject></svg>",
    "<P CLASS=a>x</P>",
    # Cut off at the end of the input, and backtracking.
    "",
    "<a title= \"unterminated>t</a>",
    "a</b x='>'c",
    "a</ x",
    "a&#",
    "a&#q",
    "a&",
    "x&amp",
    "x&am",
    "x&a-b",
    "x&a.b-c",
]

TAGS = ["p", "a", "b", "i", "script", "style", "textarea", "title", "rt", "rp",
        "template", "pre", "xmp", "noembed", "iframe", "plaintext", "div", "span"]


def case(html):
    soup = BeautifulSoup(html, "html.parser")
    out = {
        "html": html,
        "text": soup.get_text(),
        "stripped": soup.get_text("|", strip=True),
        "tags": [],
    }
    for name in TAGS:
        tag = soup.find(name)
        if tag is None:
            continue
        attrs = {k: v for k, v in tag.attrs.items() if isinstance(v, str)}
        out["tags"].append({"name": name, "text": tag.get_text(), "attrs": attrs})
    return out


def main():
    result = {
        "generator": f"Python {platform.python_version()}, beautifulsoup4 {bs4.__version__}, html.parser: html_parity.py",
        "cases": [case(h) for h in CASES],
    }
    lines = [json.dumps(c, ensure_ascii=False) for c in result["cases"]]
    sys.stdout.write(
        '{"generator": ' + json.dumps(result["generator"]) + ',\n "cases": [\n'
        + ",\n".join(lines) + "\n]}\n"
    )


main()
