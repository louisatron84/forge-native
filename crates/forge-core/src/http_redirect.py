    class _SameOriginRedirect(urllib.request.HTTPRedirectHandler):
        def redirect_request(self, req, fp, code, msg, headers, newurl):
            def origin(url):
                parsed = urllib.parse.urlsplit(url)
                port = parsed.port if parsed.port is not None else (443 if parsed.scheme == "https" else 80)
                return parsed.scheme, parsed.hostname, port
            if origin(req.full_url) != origin(newurl):
                raise RuntimeError("Cross-origin redirect blocked to keep request data private")
            return super().redirect_request(req, fp, code, msg, headers, newurl)
    urlopen = urllib.request.build_opener(_SameOriginRedirect()).open
