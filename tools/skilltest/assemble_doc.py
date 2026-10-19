"""Assembles docs/research/skill-functional.md: the hand written text (data/extract/skilltest/doc_head.md, doc_mid.md) around the tables of report.py.

usage: python tools/skilltest/assemble_doc.py
"""
import subprocess
import sys

D = "data/extract/skilltest/"
tables = subprocess.run([sys.executable, "tools/skilltest/report.py"], capture_output=True, text=True, encoding="utf-8", check=True).stdout
summary, rest = tables.split("\n\n### 1.", 1)
head = open(D + "doc_head.md", encoding="utf-8").read()
mid = open(D + "doc_mid.md", encoding="utf-8").read()
trees, defaults = rest.split("\n\n### Default skills", 1)
doc = head + summary + "\n" + mid + "### 1." + trees + "\n\n### Default skills" + defaults + "\n"
open("docs/research/skill-functional.md", "w", encoding="utf-8", newline="\n").write(doc)
print(len(doc.splitlines()), "lines")
