def _plain(value: Any) -> Any:
    """Models and lists of them as plain JSON values."""
    if hasattr(value, "to_dict"):
        return value.to_dict()
    if isinstance(value, list):
        return [_plain(v) for v in value]
    if isinstance(value, dict):
        return {k: _plain(v) for k, v in value.items()}
    return value


def _path(template: str, values: dict) -> str:
    return template.format_map({k: urllib.parse.quote(str(v), safe="") for k, v in values.items()})


def _request(method: str, path: str, params: Optional[dict] = None, headers: Optional[dict] = None, body: Any = None, form: bool = False) -> Any:
    query = {k: v for k, v in (params or {}).items() if v is not None}
    sent = {"Accept": "application/json"}
    sent.update({k: str(v) for k, v in (headers or {}).items() if v is not None})
    _auth(sent, query)
    url = BASE_URL.rstrip("/") + path
    if query:
        url += "?" + urllib.parse.urlencode(query, doseq=True)
    data = None
    if body is not None:
        if form:
            data = urllib.parse.urlencode(_plain(body), doseq=True).encode("utf-8")
            sent["Content-Type"] = "application/x-www-form-urlencoded"
        else:
            data = json.dumps(_plain(body)).encode("utf-8")
            sent["Content-Type"] = "application/json"
    request = urllib.request.Request(url, data=data, headers=sent, method=method)
    with urllib.request.urlopen(request, timeout=30) as response:
        raw = response.read()
    if not raw:
        return None
    try:
        return json.loads(raw)
    except ValueError:
        return raw.decode("utf-8", "replace")
