# forge:blocks 1
"""Forge's built-in pipeline blocks: ordinary Python functions, using pandas.

Forge keeps this file up to date. To change a block, choose Edit on it in
Forge's Pipeline view: the block is copied into your project's blocks.py
first, so updates to this file never overwrite your version.
"""
from __future__ import annotations

import json
import time
import urllib.parse
import urllib.request
from typing import Any, Callable, Optional

import pandas as pd


def block(fn):
    """Mark a function as a pipeline block: Forge lists it in the block library."""
    fn.__forge_block__ = True
    return fn


@block
def read_csv(path: str) -> pd.DataFrame:
    """Read a CSV file into a table."""
    return pd.read_csv(path)


@block
def read_json(path: str, records_path: Optional[str] = None) -> pd.DataFrame:
    """Read a JSON file (a list of records, or records under a key such as "data.items") into a table."""
    with open(path, encoding="utf-8") as f:
        data = json.load(f)
    return _records_table(data, records_path)


@block
def call_api(url: str, params: Optional[dict] = None, records_path: Optional[str] = None) -> pd.DataFrame:
    """GET a JSON web API and turn the records it returns into a table."""
    if params:
        url += ("&" if "?" in url else "?") + urllib.parse.urlencode(params, doseq=True)
    request = urllib.request.Request(url, headers={"Accept": "application/json"})
    with urllib.request.urlopen(request, timeout=30) as response:
        data = json.loads(response.read())
    return _records_table(data, records_path)


@block
def filter_rows(table: pd.DataFrame, condition: str) -> pd.DataFrame:
    """Keep the rows where the condition holds, e.g. "amount > 100 and status == 'paid'"."""
    return table.query(condition).reset_index(drop=True)


@block
def select_columns(table: pd.DataFrame, columns: list[str]) -> pd.DataFrame:
    """Keep only these columns, in this order."""
    return table[list(columns)]


@block
def rename_columns(table: pd.DataFrame, names: dict[str, str]) -> pd.DataFrame:
    """Rename columns: {"old name": "new name"}."""
    return table.rename(columns=names)


@block
def add_column(table: pd.DataFrame, name: str, expression: str) -> pd.DataFrame:
    """Add (or replace) a column worked out from others, e.g. "price * quantity"."""
    out = table.copy()
    out[name] = out.eval(expression)
    return out


@block
def sort_rows(table: pd.DataFrame, by: list[str], descending: bool = False) -> pd.DataFrame:
    """Sort the rows by these columns."""
    return table.sort_values(list(by), ascending=not descending).reset_index(drop=True)


@block
def group_by(table: pd.DataFrame, by: list[str], totals: dict[str, str]) -> pd.DataFrame:
    """Group the rows and total columns, e.g. by ["region"] with totals {"amount": "sum"}."""
    return table.groupby(list(by), as_index=False).agg(totals)


@block
def merge(left: pd.DataFrame, right: pd.DataFrame, on: list[str], how: str = "inner") -> pd.DataFrame:
    """Join two tables on matching columns (how: "inner", "left", "right" or "outer")."""
    return left.merge(right, on=list(on), how=how)


@block
def drop_duplicates(table: pd.DataFrame, columns: Optional[list[str]] = None) -> pd.DataFrame:
    """Remove repeated rows (judged on these columns, or all of them)."""
    return table.drop_duplicates(subset=columns).reset_index(drop=True)


@block
def fill_missing(table: pd.DataFrame, value: Any = 0) -> pd.DataFrame:
    """Fill empty cells with a value."""
    return table.fillna(value)


@block
def retry(step: Callable, *args: Any, attempts: int = 3, wait_seconds: float = 1.0, **kwargs: Any) -> Any:
    """Run a step, trying again if it fails (e.g. a flaky web API), waiting a little longer each time."""
    for attempt in range(1, attempts + 1):
        try:
            return step(*args, **kwargs)
        except Exception:
            if attempt == attempts:
                raise
            time.sleep(wait_seconds * attempt)


@block
def save_csv(table: pd.DataFrame, path: str) -> str:
    """Save a table as a CSV file; returns the path."""
    table.to_csv(path, index=False)
    return path


@block
def save_json(table: pd.DataFrame, path: str) -> str:
    """Save a table as a JSON list of records; returns the path."""
    table.to_json(path, orient="records", indent=2)
    return path


def _records_table(data: Any, records_path: Optional[str] = None) -> pd.DataFrame:
    """The records in some JSON (at records_path, or the largest list in it) as a table."""
    if records_path:
        for key in records_path.split("."):
            data = data[key]
    elif isinstance(data, dict):
        lists = [v for v in data.values() if isinstance(v, list)]
        if lists:
            data = max(lists, key=len)
    return pd.json_normalize(data)
