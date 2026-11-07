"""Import this first in a test module: puts tools/skillparity on sys.path and gives the fixture locations."""
import sys
from pathlib import Path

TOOLS = Path(__file__).resolve().parents[1]
FIXTURES = Path(__file__).resolve().parent / "fixtures"
RUN1 = FIXTURES / "run1"
if str(TOOLS) not in sys.path:
    sys.path.insert(0, str(TOOLS))
