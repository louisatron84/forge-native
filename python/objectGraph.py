
def forge_objects(ns, max_nodes=40, max_items=8, max_depth=3):
    import json, types
    prim = (int, float, complex, bool, str, bytes, type(None))
    skip = (types.ModuleType, types.FunctionType, types.BuiltinFunctionType, types.MethodType, type)
    nodes = {}
    edges = []

    def short(v):
        try:
            r = repr(v)
        except Exception:
            r = "<%s>" % type(v).__name__
        return r if len(r) <= 40 else r[:39] + "…"

    def visit(obj, depth):
        key = id(obj)
        if key in nodes:
            return key
        if len(nodes) >= max_nodes:
            return None
        node = {"id": str(key), "type": type(obj).__name__, "fields": [], "more": 0}
        nodes[key] = node
        children = []
        if isinstance(obj, dict):
            items = list(obj.items())
            node["len"] = len(items)
            children = [(k if isinstance(k, str) else short(k), v) for k, v in items[:max_items]]
            node["more"] = max(0, len(items) - max_items)
        elif isinstance(obj, (list, tuple, set, frozenset)):
            items = list(obj)
            node["len"] = len(items)
            ordered = isinstance(obj, (list, tuple))
            children = [("[%d]" % i if ordered else "·", v) for i, v in enumerate(items[:max_items])]
            node["more"] = max(0, len(items) - max_items)
        else:
            attrs = getattr(obj, "__dict__", None)
            if isinstance(attrs, dict):
                pairs = list(attrs.items())
            else:
                slots = getattr(type(obj), "__slots__", ())
                slots = (slots,) if isinstance(slots, str) else slots
                pairs = [(s, getattr(obj, s)) for s in slots if hasattr(obj, s)]
            children = pairs[:16]
            node["more"] = max(0, len(pairs) - 16)
            if not pairs:
                node["fields"].append(["", short(obj)])
        for name, value in children:
            if isinstance(value, prim) or isinstance(value, skip) or depth >= max_depth:
                node["fields"].append([str(name), short(value)])
                continue
            target = visit(value, depth + 1)
            if target is None:
                node["fields"].append([str(name), short(value)])
            else:
                edges.append([str(key), str(target), str(name)])
        return key

    roots = []
    for name, value in list(ns.items()):
        if name.startswith("__") or isinstance(value, prim) or isinstance(value, skip):
            continue
        key = visit(value, 0)
        if key is not None:
            roots.append([name, str(key)])
    return json.dumps({"roots": roots, "nodes": list(nodes.values()), "edges": edges}, ensure_ascii=True)
