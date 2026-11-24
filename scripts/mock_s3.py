#!/usr/bin/env python3
"""mock_s3.py - a tiny S3-compatible server for testing Passman's backup targets.

DEVELOPMENT ONLY. Python standard library only. Listens on 127.0.0.1 and nowhere else.

It is a *verifying* stand-in, not a permissive one: every request must carry a valid AWS
Signature Version 4 **query-string** signature (what Passman's S3 client sends), computed
independently here from the canonical request. It rejects a wrong access key, a wrong secret
key, a wrong region and a clock that is too far off, exactly as the answers an S3 service gives
(InvalidAccessKeyId, SignatureDoesNotMatch, AuthorizationQueryParametersError,
RequestTimeTooSkewed), so the real client's error handling is exercised with real signed
requests.

Supported (path-style: /<bucket>/<key>):
  PUT     object, with  If-None-Match: *  and  If-Match: <etag>  (412 / 404 like S3)
  GET     object            HEAD    object            DELETE  object (always 204)
  GET     /<bucket>?list-type=2&prefix=&continuation-token=&max-keys=&encoding-type=url
          (ListObjectsV2 with continuation tokens)

Stricter than S3 on purpose: a request that carries If-Match / If-None-Match must have signed
them (they are in X-Amz-SignedHeaders), which proves the client signs its preconditions.

Failure injection (to watch health states change):
  --fail-after N        after N requests, answer every request with --fail-status
  --fail-status 403|500|503
  POST /__mock/fail     {"status": 403|500|503}  start failing now;  {"status": null}  recover;
                        {"status": 500, "after": 3}  fail from the 4th request on
  POST /__mock/reset    forget every object and counter, stop failing
  GET  /__mock/state    {"requests": n, "failing": 0|status, "objects": n, "keys": [...]}
The /__mock/ endpoints refuse requests that carry an Origin header (a web page cannot reach
them) and need Content-Type: application/json for POST.

Example:
  python3 scripts/mock_s3.py --port 9333 --access-key AKIAMOCK --secret-key mocksecret \\
      --region us-east-1 --bucket passman-test
  python3 scripts/mock_s3.py --selftest        # SigV4 known-answer test, then exit

With --port 0 the OS picks a free port; the first stdout line is then  MOCK_S3_PORT=<n>.
Secrets are never printed. Request logging (--verbose) prints method, path and status only,
never the query string (it contains the signature).
"""

import argparse
import base64
import datetime
import hashlib
import hmac
import json
import os
import sys
import tempfile
import threading
import time
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

MAX_SKEW_SECONDS = 15 * 60
MAX_EXPIRES = 7 * 24 * 3600
XML_NS = "http://s3.amazonaws.com/doc/2006-03-01/"


# ---------------------------------------------------------------------------------------
# SigV4 (query-string authentication), written from the AWS documentation, independent of
# any S3 library.

def rfc3986(text):
    """Percent-encode everything except the RFC 3986 unreserved characters."""
    return urllib.parse.quote(text, safe="-_.~")


def parse_query(raw):
    """Raw query string -> [(decoded name, decoded value)]; '+' is a literal plus here."""
    pairs = []
    for part in raw.split("&") if raw else []:
        if not part:
            continue
        name, _, value = part.partition("=")
        pairs.append((urllib.parse.unquote(name), urllib.parse.unquote(value)))
    return pairs


def canonical_query(pairs):
    encoded = sorted((rfc3986(k), rfc3986(v)) for k, v in pairs if k != "X-Amz-Signature")
    return "&".join(f"{k}={v}" for k, v in encoded)


def canonical_path(raw_path):
    """S3 canonical URI: the decoded path, encoded once with the unreserved set (and '/')."""
    return urllib.parse.quote(urllib.parse.unquote(raw_path), safe="/-_.~")


def signing_key(secret, date, region):
    def h(key, msg):
        return hmac.new(key, msg.encode(), hashlib.sha256).digest()
    k = h(("AWS4" + secret).encode(), date)
    k = h(k, region)
    k = h(k, "s3")
    return h(k, "aws4_request")


def sign_query(secret, method, raw_path, pairs, signed_headers, header_values, amz_date, region):
    """The hex signature of a query-signed request (payload is always UNSIGNED-PAYLOAD)."""
    names = signed_headers.split(";")
    block = "".join(f"{n}:{' '.join(header_values[n].split())}\n" for n in names)
    canonical = "\n".join([
        method,
        canonical_path(raw_path),
        canonical_query(pairs),
        block,
        signed_headers,
        "UNSIGNED-PAYLOAD",
    ])
    date = amz_date[:8]
    to_sign = "\n".join([
        "AWS4-HMAC-SHA256",
        amz_date,
        f"{date}/{region}/s3/aws4_request",
        hashlib.sha256(canonical.encode()).hexdigest(),
    ])
    key = signing_key(secret, date, region)
    return hmac.new(key, to_sign.encode(), hashlib.sha256).hexdigest()


def selftest():
    """AWS documentation example (presigned GET, 'examplebucket/test.txt')."""
    pairs = [
        ("X-Amz-Algorithm", "AWS4-HMAC-SHA256"),
        ("X-Amz-Credential", "AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request"),
        ("X-Amz-Date", "20130524T000000Z"),
        ("X-Amz-Expires", "86400"),
        ("X-Amz-SignedHeaders", "host"),
    ]
    got = sign_query(
        "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY", "GET", "/test.txt", pairs, "host",
        {"host": "examplebucket.s3.amazonaws.com"}, "20130524T000000Z", "us-east-1",
    )
    want = "aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404"
    assert got == want, f"SigV4 known-answer test FAILED: {got} != {want}"
    assert canonical_path("/a b/ü~.txt") == "/a%20b/%C3%BC~.txt"
    assert canonical_query([("b", "2"), ("a", "x y"), ("X-Amz-Signature", "zz")]) == "a=x%20y&b=2"
    print("selftest 1/2 ok: SigV4 matches the AWS documentation example")
    serve_selftest()
    print("selftest 2/2 ok: the server accepts good signatures and rejects bad ones")


def serve_selftest():
    """Start the server on a free port and talk to it with independently signed requests."""
    import http.client
    import re

    ns = argparse.Namespace(
        access_key="AKIATEST", secret_key="test-secret", region="eu-west-1", bucket="bk",
        data_dir=None, fail_after=None, fail_status=500, page_size=2, max_object_mib=1,
        verbose=False)
    Handler.state = State(ns)
    srv = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    srv.daemon_threads = True
    port = srv.server_address[1]
    threading.Thread(target=srv.serve_forever, daemon=True).start()

    def call(method, path, extra=(), body=b"", key=None, secret=None, region=None,
             skew=0, headers=None, sign_headers=True, expires=900):
        key = key or ns.access_key
        secret = secret or ns.secret_key
        region = region or ns.region
        when = datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(seconds=skew)
        amz = when.strftime("%Y%m%dT%H%M%SZ")
        headers = dict(headers or {})
        names = sorted(["host"] + ([h.lower() for h in headers] if sign_headers else []))
        values = {"host": f"127.0.0.1:{port}"}
        values.update({k.lower(): v for k, v in headers.items()})
        pairs = [("X-Amz-Algorithm", "AWS4-HMAC-SHA256"),
                 ("X-Amz-Credential", f"{key}/{amz[:8]}/{region}/s3/aws4_request"),
                 ("X-Amz-Date", amz), ("X-Amz-Expires", str(expires)),
                 ("X-Amz-SignedHeaders", ";".join(names))] + list(extra)
        sig = sign_query(secret, method, path, pairs, ";".join(names), values, amz, region)
        query = "&".join(f"{rfc3986(k)}={rfc3986(v)}" for k, v in pairs) + f"&X-Amz-Signature={sig}"
        conn = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
        conn.request(method, f"{urllib.parse.quote(path, safe='/-_.~')}?{query}", body=body or None,
                     headers=headers)
        resp = conn.getresponse()
        data = resp.read()
        conn.close()
        code = re.search(rb"<Code>(\w+)</Code>", data)
        return resp.status, (code.group(1).decode() if code else None), data, resp

    def control(path, payload=None, method="POST"):
        conn = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
        conn.request(method, path, body=json.dumps(payload).encode() if payload is not None else None,
                     headers={"Content-Type": "application/json"} if method == "POST" else {})
        r = conn.getresponse()
        data = r.read()
        conn.close()
        return r.status, data

    def expect(got, want, what):
        assert got[:2] == want, f"{what}: got {got[:2]}, wanted {want}"

    # object round trip
    expect(call("PUT", "/bk/a b/one.txt", body=b"hello"), (200, None), "put")
    st, _, data, resp = call("GET", "/bk/a b/one.txt")
    assert (st, data) == (200, b"hello") and resp.getheader("ETag") == etag_of(b"hello"), "get"
    expect(call("HEAD", "/bk/a b/one.txt"), (200, None), "head")
    expect(call("GET", "/bk/missing"), (404, "NoSuchKey"), "missing key")
    expect(call("DELETE", "/bk/a b/one.txt"), (204, None), "delete")
    expect(call("DELETE", "/bk/a b/one.txt"), (204, None), "delete again")
    expect(call("GET", "/nobucket/x"), (404, "NoSuchBucket"), "wrong bucket")

    # preconditions
    h = {"If-None-Match": "*"}
    expect(call("PUT", "/bk/k", body=b"1", headers=h), (200, None), "create")
    expect(call("PUT", "/bk/k", body=b"2", headers=h), (412, "PreconditionFailed"), "create twice")
    expect(call("PUT", "/bk/k", body=b"2", headers={"If-Match": '"nope"'}),
           (412, "PreconditionFailed"), "wrong etag")
    expect(call("PUT", "/bk/k", body=b"2", headers={"If-Match": etag_of(b"1")}), (200, None), "right etag")
    expect(call("PUT", "/bk/none", body=b"2", headers={"If-Match": etag_of(b"1")}),
           (404, "NoSuchKey"), "If-Match on a missing key")
    expect(call("PUT", "/bk/k", body=b"3", headers=h, sign_headers=False)[:2],
           (403, "AccessDenied"), "unsigned precondition")

    # authentication
    expect(call("GET", "/bk/k", secret="wrong"), (403, "SignatureDoesNotMatch"), "wrong secret")
    expect(call("GET", "/bk/k", key="AKIAWRONG"), (403, "InvalidAccessKeyId"), "wrong key")
    expect(call("GET", "/bk/k", region="us-east-1"), (400, "AuthorizationQueryParametersError"), "wrong region")
    expect(call("GET", "/bk/k", skew=3600), (403, "RequestTimeTooSkewed"), "clock ahead")
    expect(call("GET", "/bk/k", skew=-3600), (403, "AccessDenied"), "clock behind (expired)")
    expect(call("GET", "/bk/k", skew=-3600, expires=7200), (200, None), "old but within expires")
    conn = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
    conn.request("GET", "/bk/k")
    assert conn.getresponse().status == 403, "unsigned request"
    conn.close()

    # listing with continuation tokens and url encoding
    for name in ["p/3", "p/1", "p/2", "p/with space", "q/0"]:
        call("PUT", f"/bk/{name}", body=b"x")
    seen, token, pages = [], None, 0
    while True:
        extra = [("list-type", "2"), ("prefix", "p/"), ("encoding-type", "url")]
        if token:
            extra.append(("continuation-token", token))
        st, _, data, _ = call("GET", "/bk", extra=extra)
        assert st == 200, data
        text = data.decode()
        seen += [urllib.parse.unquote_plus(k) for k in re.findall(r"<Key>(.*?)</Key>", text)]
        pages += 1
        m = re.search(r"<NextContinuationToken>(.*?)</NextContinuationToken>", text)
        if not m:
            break
        token = m.group(1)
    assert seen == ["p/1", "p/2", "p/3", "p/with space"] and pages == 2, (seen, pages)

    # failure injection
    assert control("/__mock/fail", {"status": 403})[0] == 200
    expect(call("GET", "/bk/k"), (403, "AccessDenied"), "injected 403")
    assert control("/__mock/fail", {"status": None})[0] == 200
    expect(call("GET", "/bk/k"), (200, None), "recovered")
    assert control("/__mock/fail", {"status": 500, "after": 1})[0] == 200
    expect(call("GET", "/bk/k"), (200, None), "one more request before failing")
    expect(call("GET", "/bk/k"), (500, "InternalError"), "then 500")
    assert control("/__mock/reset", {})[0] == 200
    expect(call("GET", "/bk/k"), (404, "NoSuchKey"), "reset forgot the objects")
    assert control("/__mock/fail", {"status": 418})[0] == 400
    srv.shutdown()


# ---------------------------------------------------------------------------------------
# object storage

class ObjectStore:
    """Objects in memory, or in --data-dir (one file per object, atomic writes)."""

    def __init__(self, data_dir):
        self.mem = {}
        self.dir = data_dir
        if data_dir:
            os.makedirs(data_dir, exist_ok=True)

    def _path(self, key):
        return os.path.join(self.dir, key.encode().hex())

    def put(self, key, body):
        if self.dir:
            fd, tmp = tempfile.mkstemp(dir=self.dir, prefix=".tmp-")
            with os.fdopen(fd, "wb") as f:
                f.write(body)
            os.replace(tmp, self._path(key))
        else:
            self.mem[key] = body

    def get(self, key):
        if self.dir:
            try:
                with open(self._path(key), "rb") as f:
                    return f.read()
            except FileNotFoundError:
                return None
        return self.mem.get(key)

    def delete(self, key):
        if self.dir:
            try:
                os.remove(self._path(key))
            except FileNotFoundError:
                pass
        else:
            self.mem.pop(key, None)

    def keys(self):
        if self.dir:
            out = []
            for name in os.listdir(self.dir):
                if name.startswith(".tmp-"):
                    continue
                try:
                    out.append(bytes.fromhex(name).decode())
                except ValueError:
                    pass
            return sorted(out)
        return sorted(self.mem)

    def clear(self):
        for k in self.keys():
            self.delete(k)


def etag_of(body):
    return '"' + hashlib.md5(body).hexdigest() + '"'


def bare(etag):
    etag = etag.strip()
    if etag.startswith("W/"):
        etag = etag[2:]
    return etag.strip('"')


class State:
    def __init__(self, args):
        self.args = args
        self.store = ObjectStore(args.data_dir)
        self.lock = threading.Lock()
        self.requests = 0
        self.fail_status = args.fail_status if args.fail_after is not None else 0
        self.fail_after = args.fail_after

    def tick(self):
        """Count a request; the status to fail it with, or 0."""
        with self.lock:
            self.requests += 1
            if self.fail_after is not None and self.requests > self.fail_after:
                return self.fail_status
            return 0


class Reject(Exception):
    def __init__(self, status, code, message):
        super().__init__(message)
        self.status, self.code, self.message = status, code, message


def xml_escape(text):
    return (text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
            .replace('"', "&quot;"))


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "mock-s3"
    state = None  # set on the class

    timeout = 30  # an idle keep-alive connection is dropped after this many seconds

    # ---- plumbing
    def handle(self):
        try:
            super().handle()
        except (ConnectionError, TimeoutError):
            pass  # the client went away

    def log_message(self, fmt, *args):  # silence the default (it logs the query string)
        pass

    def _log(self, status):
        if self.state.args.verbose:
            path = self.path.split("?", 1)[0]
            sys.stderr.write(f"{self.command} {path} -> {status}\n")

    def _reply(self, status, body=b"", headers=None, content_type="application/xml", head=False):
        self.send_response(status)
        self.send_header("Content-Length", str(len(body)))
        if body or head:
            self.send_header("Content-Type", content_type)
        for k, v in (headers or {}).items():
            self.send_header(k, v)
        self.send_header("x-amz-request-id", "mock")
        self.end_headers()
        if not head:
            self.wfile.write(body)
        self._log(status)

    def _error(self, status, code, message, head=False):
        resource = xml_escape(self.path.split("?", 1)[0])
        body = (f'<?xml version="1.0" encoding="UTF-8"?><Error><Code>{code}</Code>'
                f"<Message>{xml_escape(message)}</Message><Resource>{resource}</Resource>"
                f"<RequestId>mock</RequestId></Error>").encode()
        self._reply(status, b"" if head else body, head=head)

    def _read_body(self):
        length = self.headers.get("Content-Length")
        if length is None:
            if self.headers.get("Transfer-Encoding"):
                raise Reject(411, "MissingContentLength", "Chunked uploads are not supported.")
            return b""
        try:
            n = int(length)
        except ValueError:
            raise Reject(400, "InvalidArgument", "Bad Content-Length.")
        if n < 0 or n > self.state.args.max_object_mib * 1024 * 1024:
            self.close_connection = True
            raise Reject(413, "EntityTooLarge", "Your proposed upload exceeds the limit.")
        return self.rfile.read(n)

    # ---- dispatch
    def do_GET(self):
        self._handle()

    def do_HEAD(self):
        self._handle()

    def do_PUT(self):
        self._handle()

    def do_DELETE(self):
        self._handle()

    def do_POST(self):
        self._handle()

    def _handle(self):
        head = self.command == "HEAD"
        try:
            raw_path, _, raw_query = self.path.partition("?")
            if raw_path.startswith("/__mock/"):
                return self._control(raw_path)
            self._check_host()
            body = self._read_body() if self.command == "PUT" else b""
            forced = self.state.tick()
            if forced:
                codes = {403: "AccessDenied", 500: "InternalError", 503: "SlowDown"}
                raise Reject(forced, codes.get(forced, "InternalError"), "Injected failure.")
            self._authenticate(raw_path, raw_query)
            self._route(raw_path, raw_query, body)
        except Reject as r:
            self._error(r.status, r.code, r.message, head=head)
        except (BrokenPipeError, ConnectionResetError):
            pass

    def _check_host(self):
        host = self.headers.get("Host", "")
        port = self.server.server_address[1]
        if host not in (f"127.0.0.1:{port}", f"localhost:{port}"):
            raise Reject(403, "AccessDenied", "Unexpected Host header.")

    # ---- authentication
    def _authenticate(self, raw_path, raw_query):
        a = self.state.args
        pairs = parse_query(raw_query)
        q = {}
        for k, v in pairs:
            q.setdefault(k, v)
        if q.get("X-Amz-Algorithm") != "AWS4-HMAC-SHA256" or "X-Amz-Signature" not in q:
            raise Reject(403, "AccessDenied",
                         "Only AWS Signature Version 4 query authentication is accepted.")
        cred = q.get("X-Amz-Credential", "").split("/")
        if len(cred) != 5 or cred[3] != "s3" or cred[4] != "aws4_request":
            raise Reject(400, "AuthorizationQueryParametersError", "Error parsing the X-Amz-Credential parameter.")
        key_id, scope_date, region = cred[0], cred[1], cred[2]
        amz_date = q.get("X-Amz-Date", "")
        try:
            when = datetime.datetime.strptime(amz_date, "%Y%m%dT%H%M%SZ").replace(
                tzinfo=datetime.timezone.utc)
        except ValueError:
            raise Reject(400, "AuthorizationQueryParametersError", "X-Amz-Date must be in the ISO8601 basic format.")
        if scope_date != amz_date[:8]:
            raise Reject(400, "AuthorizationQueryParametersError", "The credential date does not match X-Amz-Date.")
        if key_id != a.access_key:
            raise Reject(403, "InvalidAccessKeyId",
                         "The AWS Access Key Id you provided does not exist in our records.")
        if region != a.region:
            raise Reject(400, "AuthorizationQueryParametersError",
                         f"Error parsing the X-Amz-Credential parameter; the region '{region}' is wrong; expecting '{a.region}'")
        try:
            expires = int(q.get("X-Amz-Expires", ""))
        except ValueError:
            raise Reject(400, "AuthorizationQueryParametersError", "X-Amz-Expires should be a number.")
        if expires < 1 or expires > MAX_EXPIRES:
            raise Reject(400, "AuthorizationQueryParametersError", "X-Amz-Expires must be between 1 and 604800.")
        now = datetime.datetime.now(datetime.timezone.utc)
        delta = (now - when).total_seconds()  # > 0: the request was signed in the past
        if delta < -MAX_SKEW_SECONDS:
            raise Reject(403, "RequestTimeTooSkewed",
                         "The difference between the request time and the current time is too large.")
        if delta > expires:
            # what S3 answers for a signature past its X-Amz-Expires window
            raise Reject(403, "AccessDenied", "Request has expired")
        signed = q.get("X-Amz-SignedHeaders", "")
        names = signed.split(";") if signed else []
        if "host" not in names or names != sorted(names):
            raise Reject(400, "AuthorizationQueryParametersError", "X-Amz-SignedHeaders must be sorted and include host.")
        values = {}
        for n in names:
            v = self.headers.get(n)
            if v is None:
                raise Reject(403, "SignatureDoesNotMatch", "A signed header is missing from the request.")
            values[n] = v
        want = sign_query(a.secret_key, self.command, raw_path, pairs, signed, values, amz_date, region)
        if not hmac.compare_digest(want, q["X-Amz-Signature"]):
            raise Reject(403, "SignatureDoesNotMatch",
                         "The request signature we calculated does not match the signature you provided.")
        # stricter than S3: preconditions must be covered by the signature
        for h in ("if-none-match", "if-match"):
            if self.headers.get(h) is not None and h not in names:
                raise Reject(403, "AccessDenied", f"The {h} header is not signed.")

    # ---- the S3 API
    def _route(self, raw_path, raw_query, body):
        a = self.state.args
        path = urllib.parse.unquote(raw_path)
        parts = path.lstrip("/").split("/", 1)
        bucket = parts[0]
        key = parts[1] if len(parts) > 1 else ""
        if bucket != a.bucket:
            raise Reject(404, "NoSuchBucket", "The specified bucket does not exist.")
        if not key:
            if self.command == "GET":
                return self._list(parse_query(raw_query))
            raise Reject(405, "MethodNotAllowed", "Unsupported bucket operation.")
        with self.state.lock:
            store = self.state.store
            if self.command == "PUT":
                return self._put(store, key, body)
            if self.command in ("GET", "HEAD"):
                data = store.get(key)
                if data is None:
                    raise Reject(404, "NoSuchKey", "The specified key does not exist.")
                head = self.command == "HEAD"
                return self._reply(200, data, {"ETag": etag_of(data)},
                                   content_type="application/octet-stream", head=head)
            if self.command == "DELETE":
                store.delete(key)
                return self._reply(204)
        raise Reject(405, "MethodNotAllowed", "Unsupported operation.")

    def _put(self, store, key, body):
        none_match = self.headers.get("If-None-Match")
        if_match = self.headers.get("If-Match")
        current = store.get(key)
        if none_match is not None:
            if none_match.strip() != "*":
                raise Reject(400, "InvalidArgument", "Only If-None-Match: * is supported on writes.")
            if current is not None:
                raise Reject(412, "PreconditionFailed",
                             "At least one of the pre-conditions you specified did not hold.")
        if if_match is not None:
            if current is None:
                raise Reject(404, "NoSuchKey", "The specified key does not exist.")
            if bare(if_match) != bare(etag_of(current)):
                raise Reject(412, "PreconditionFailed",
                             "At least one of the pre-conditions you specified did not hold.")
        store.put(key, body)
        self._reply(200, b"", {"ETag": etag_of(body)})

    def _list(self, pairs):
        q = {}
        for k, v in pairs:
            q.setdefault(k, v)
        if q.get("list-type") != "2":
            raise Reject(400, "InvalidArgument", "Only ListObjectsV2 (list-type=2) is supported.")
        prefix = q.get("prefix", "")
        try:
            max_keys = int(q.get("max-keys", "1000"))
        except ValueError:
            raise Reject(400, "InvalidArgument", "max-keys must be a number.")
        max_keys = max(1, min(max_keys, 1000, self.state.args.page_size))
        after = ""
        token = q.get("continuation-token")
        if token:
            try:
                after = base64.urlsafe_b64decode(token.encode() + b"==").decode()
            except Exception:
                raise Reject(400, "InvalidArgument", "The continuation token provided is incorrect.")
        elif q.get("start-after"):
            after = q["start-after"]
        url_encoding = q.get("encoding-type") == "url"

        def show(k):
            return urllib.parse.quote_plus(k, safe="/") if url_encoding else k

        with self.state.lock:
            store = self.state.store
            keys = [k for k in store.keys() if k.startswith(prefix) and k > after]
            page = keys[:max_keys]
            truncated = len(keys) > max_keys
            rows = []
            for k in page:
                data = store.get(k) or b""
                rows.append(
                    f"<Contents><Key>{xml_escape(show(k))}</Key>"
                    f"<LastModified>2025-01-01T00:00:00.000Z</LastModified>"
                    f"<ETag>{xml_escape(etag_of(data))}</ETag><Size>{len(data)}</Size>"
                    f"<StorageClass>STANDARD</StorageClass></Contents>")
        out = [f'<?xml version="1.0" encoding="UTF-8"?><ListBucketResult xmlns="{XML_NS}">',
               f"<Name>{xml_escape(self.state.args.bucket)}</Name>",
               f"<Prefix>{xml_escape(show(prefix))}</Prefix>",
               f"<KeyCount>{len(page)}</KeyCount><MaxKeys>{max_keys}</MaxKeys>",
               f"<IsTruncated>{'true' if truncated else 'false'}</IsTruncated>"]
        if url_encoding:
            out.append("<EncodingType>url</EncodingType>")
        if truncated:
            next_token = base64.urlsafe_b64encode(page[-1].encode()).decode().rstrip("=")
            out.append(f"<NextContinuationToken>{next_token}</NextContinuationToken>")
        out.extend(rows)
        out.append("</ListBucketResult>")
        self._reply(200, "".join(out).encode())

    # ---- /__mock/ control endpoints (development only)
    def _control(self, raw_path):
        if self.headers.get("Origin") is not None:
            raise Reject(403, "AccessDenied", "Not available to web pages.")
        self._check_host()
        s = self.state
        if self.command == "GET" and raw_path == "/__mock/state":
            with s.lock:
                payload = {"requests": s.requests,
                           "failing": s.fail_status if s.fail_after is not None else 0,
                           "failAfter": s.fail_after, "objects": len(s.store.keys()),
                           "keys": s.store.keys()}
            return self._json(payload)
        if self.command != "POST" or raw_path not in ("/__mock/fail", "/__mock/reset"):
            raise Reject(404, "NotFound", "Unknown control endpoint.")
        ctype = (self.headers.get("Content-Type") or "").lower()
        if not ctype.startswith("application/json"):
            raise Reject(415, "UnsupportedMediaType", "Send application/json.")
        raw = self._read_body()
        try:
            data = json.loads(raw or b"{}")
            if not isinstance(data, dict):
                raise ValueError
        except ValueError:
            raise Reject(400, "InvalidArgument", "Bad JSON.")
        with s.lock:
            if raw_path == "/__mock/reset":
                s.store.clear()
                s.requests = 0
                s.fail_after = None
                s.fail_status = 0
            else:
                status = data.get("status")
                if status in (None, 0):
                    s.fail_after = None
                    s.fail_status = 0
                elif status in (403, 500, 503):
                    after = data.get("after", 0)
                    if not isinstance(after, int) or isinstance(after, bool) or after < 0:
                        raise Reject(400, "InvalidArgument", "after must be a non-negative number.")
                    s.fail_status = status
                    s.fail_after = s.requests + after
                else:
                    raise Reject(400, "InvalidArgument", "status must be 403, 500, 503 or null.")
        self._json({"ok": True})

    def _json(self, payload):
        self._reply(200, json.dumps(payload).encode(), content_type="application/json")


# ---------------------------------------------------------------------------------------

def main():
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("--port", type=int, default=9333, help="0 = pick a free port (printed)")
    p.add_argument("--access-key", default="AKIAMOCKMOCKMOCK")
    p.add_argument("--secret-key", default="mock-secret-access-key-not-real")
    p.add_argument("--region", default="us-east-1")
    p.add_argument("--bucket", default="passman-test")
    p.add_argument("--data-dir", default=None, help="keep objects here (default: in memory)")
    p.add_argument("--fail-after", type=int, default=None, metavar="N",
                   help="answer every request after the first N with --fail-status")
    p.add_argument("--fail-status", type=int, choices=(403, 500, 503), default=500)
    p.add_argument("--page-size", type=int, default=1000, help="most keys per listing page")
    p.add_argument("--max-object-mib", type=int, default=128)
    p.add_argument("--verbose", action="store_true", help="log method, path and status to stderr")
    p.add_argument("--selftest", action="store_true", help="run the SigV4 known-answer test and exit")
    args = p.parse_args()
    if args.selftest:
        selftest()
        return
    if args.fail_after is not None and args.fail_after < 0:
        p.error("--fail-after must be 0 or more")
    if args.page_size < 1:
        p.error("--page-size must be at least 1")
    Handler.state = State(args)
    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    server.daemon_threads = True
    port = server.server_address[1]
    print(f"MOCK_S3_PORT={port}", flush=True)
    print(f"mock S3 on http://127.0.0.1:{port}  bucket={args.bucket} region={args.region} "
          f"access-key={args.access_key} (path-style, development only)", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
