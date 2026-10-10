#!/usr/bin/env python3
"""A stand-in for Google's OAuth, Gmail and Calendar APIs, for the mail and
calendar soak (README.md). A `vak` built with `vak-server/test-support`
sends every provider call here when `VAK_TEST_PROVIDER_BASE` names it.

It speaks only the requests the adapters make, signs id tokens with a key
of its own (RS256, through the `openssl` command), keeps its state in a
file so it survives being stopped, and takes faults from the soak through
`/_soak/*`. Throwaway values only: nothing here is a real credential.
"""
import base64, hashlib, json, os, random, secrets, subprocess, sys, threading, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlencode, urlparse

PORT = int(os.environ.get("FAKE_PORT", "9100"))
HOME = os.environ.get("FAKE_HOME", "/data/fake")
STATE = os.path.join(HOME, "state.json")
KEY = os.path.join(HOME, "signing.pem")
LOCK = threading.Lock()
ACCESS_SECONDS = int(os.environ.get("FAKE_ACCESS_SECONDS", "900"))


def b64u(data):
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode()


def now():
    return time.time()


def fresh_state():
    return {
        "codes": {}, "access": {}, "refresh": {}, "history_id": 1000, "min_history": 0,
        "history": [], "messages": [], "events": [], "fault": {"mode": "ok", "until": 0},
        "counts": {}, "sends": {}, "next_message": 1, "fetched": [],
    }


def load():
    try:
        with open(STATE) as handle:
            return json.load(handle)
    except FileNotFoundError:
        return fresh_state()


S = None


def save():
    tmp = STATE + ".tmp"
    with open(tmp, "w") as handle:
        json.dump(S, handle)
    os.replace(tmp, STATE)


def count(name):
    S["counts"][name] = S["counts"].get(name, 0) + 1


def signing_key():
    if not os.path.exists(KEY):
        subprocess.run(["openssl", "genrsa", "-out", KEY, "2048"], check=True, capture_output=True)
    text = subprocess.run(["openssl", "rsa", "-in", KEY, "-noout", "-modulus"],
                          check=True, capture_output=True, text=True).stdout
    modulus = bytes.fromhex(text.strip().split("=", 1)[1])
    return {"kty": "RSA", "kid": "soak-1", "alg": "RS256", "use": "sig",
            "n": b64u(modulus), "e": b64u((65537).to_bytes(3, "big"))}


JWK = None


def sign(claims):
    header = b64u(json.dumps({"alg": "RS256", "kid": "soak-1", "typ": "JWT"}).encode())
    body = b64u(json.dumps(claims).encode())
    signed = f"{header}.{body}".encode()
    signature = subprocess.run(["openssl", "dgst", "-sha256", "-sign", KEY], input=signed,
                               check=True, capture_output=True).stdout
    return f"{header}.{body}.{b64u(signature)}"


def fault():
    mode = S["fault"]
    if mode["mode"] != "ok" and mode["until"] and now() > mode["until"]:
        S["fault"] = {"mode": "ok", "until": 0}
    return S["fault"]["mode"]


def add_message(subject=None):
    number = S["next_message"]
    S["next_message"] += 1
    message_id = f"m{number:07d}"
    S["history_id"] += 1
    S["messages"].insert(0, {
        "id": message_id, "threadId": f"t{number:07d}", "at": int(now() * 1000),
        "subject": subject or f"Soak note {number}",
        "body": f"Soak message {number}: the {random.choice(['quince', 'heliotrope', 'juniper', 'marigold'])} plan moves to {random.choice(['Monday', 'Thursday', 'Friday'])}.",
    })
    del S["messages"][500:]
    S["history"].append({"id": S["history_id"], "message": message_id})
    del S["history"][:-2000]


def gmail_message(message):
    return {
        "id": message["id"], "threadId": message["threadId"], "labelIds": ["INBOX"],
        "internalDate": str(message["at"]), "snippet": message["body"][:80],
        "payload": {
            "mimeType": "text/plain",
            "headers": [
                {"name": "From", "value": "Soak Sender <sender@example.test>"},
                {"name": "To", "value": "owner@example.test"},
                {"name": "Subject", "value": message["subject"]},
                {"name": "Date", "value": time.strftime("%a, %d %b %Y %H:%M:%S +0000", time.gmtime(message["at"] / 1000))},
            ],
            "body": {"data": b64u(message["body"].encode()), "size": len(message["body"])},
        },
    }


def iso(seconds):
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(seconds))


def parse_iso(text):
    text = text.replace("Z", "+00:00")
    from datetime import datetime
    return datetime.fromisoformat(text).timestamp()


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def reply(self, status, body=None, headers=None):
        data = b"" if body is None else (body if isinstance(body, bytes) else json.dumps(body).encode())
        self.send_response(status)
        for key, value in (headers or {}).items():
            self.send_header(key, value)
        if body is not None:
            self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def form(self):
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length).decode() if length else ""
        if self.headers.get("Content-Type", "").startswith("application/json"):
            return json.loads(raw or "{}")
        return {k: v[0] for k, v in parse_qs(raw).items()}

    def authorized(self):
        token = (self.headers.get("Authorization") or "").removeprefix("Bearer ")
        expiry = S["access"].get(token)
        return expiry is not None and expiry > now()

    def provider_fault(self):
        mode = fault()
        if mode == "error500":
            count("served_500")
            self.reply(500, {"error": {"code": 500, "message": "backend error"}})
            return True
        if mode == "error429":
            count("served_429")
            self.reply(429, {"error": {"code": 429, "message": "rate limited"}}, {"Retry-After": "30"})
            return True
        if mode == "hang":
            count("served_hang")
            time.sleep(25)
            self.reply(504, {"error": {"code": 504}})
            return True
        return False

    def do_GET(self):
        with LOCK:
            self.route("GET")

    def do_POST(self):
        with LOCK:
            self.route("POST")

    def route(self, method):
        url = urlparse(self.path)
        path, query = url.path, {k: v[0] for k, v in parse_qs(url.query).items()}
        try:
            if path.startswith("/_soak/"):
                return self.control(method, path)
            if path == "/google/authorize":
                return self.authorize(query)
            if path == "/google/token":
                return self.token()
            if path == "/google/certs":
                return self.reply(200, {"keys": [JWK]})
            if path == "/google/revoke":
                return self.reply(200, {})
            if not path.startswith(("/gmail/", "/calendar/")):
                return self.reply(404, {"error": "unknown"})
            count("api")
            if self.provider_fault():
                return
            if not self.authorized():
                count("served_401")
                return self.reply(401, {"error": {"code": 401, "status": "UNAUTHENTICATED"}})
            if path.startswith("/gmail/"):
                return self.gmail(method, path, query)
            return self.calendar(path, query)
        finally:
            save()

    def control(self, method, path):
        if path == "/_soak/stats":
            return self.reply(200, {
                "counts": S["counts"], "messages": len(S["messages"]), "events": len(S["events"]),
                "history_id": S["history_id"], "min_history": S["min_history"], "fault": S["fault"],
                "sends": S["sends"],
            })
        if path == "/_soak/unread":
            # Messages older than `older` seconds that no read has fetched.
            older = float(parse_qs(urlparse(self.path).query).get("older", ["900"])[0])
            fetched = set(S.get("fetched", []))
            unread = [m["id"] for m in S["messages"] if m["at"] / 1000 < now() - older and m["id"] not in fetched]
            return self.reply(200, {"unread": len(unread), "sample": unread[:5], "fetched": len(fetched)})
        body = self.form() if method == "POST" else {}
        if path == "/_soak/fault":
            seconds = float(body.get("seconds", 0))
            S["fault"] = {"mode": body["mode"], "until": now() + seconds if seconds else 0}
            if body["mode"] == "reject_tokens":
                S["access"].clear()
                S["fault"] = {"mode": "ok", "until": 0}
            elif body["mode"] == "revoke_refresh":
                S["refresh"].clear()
                S["access"].clear()
                S["fault"] = {"mode": "ok", "until": 0}
            elif body["mode"] == "history_reset":
                S["min_history"] = S["history_id"]
                S["fault"] = {"mode": "ok", "until": 0}
            return self.reply(200, S["fault"])
        if path == "/_soak/mail":
            for _ in range(int(body.get("count", 1))):
                add_message()
            return self.reply(200, {"messages": len(S["messages"])})
        if path == "/_soak/events":
            start = now() + float(body.get("start_in_minutes", 15)) * 60
            for index in range(int(body.get("count", 1))):
                number = len(S["events"]) + 1
                begin = start + index * float(body.get("spacing_minutes", 0)) * 60
                S["events"].append({"id": f"e{number:07d}{secrets.token_hex(2)}", "start": begin,
                                    "end": begin + 1800, "summary": f"Soak meeting {number}"})
            S["events"] = [event for event in S["events"] if event["end"] > now() - 86400][-1000:]
            return self.reply(200, {"events": len(S["events"])})
        return self.reply(404, {})

    def authorize(self, query):
        code = secrets.token_urlsafe(24)
        S["codes"][code] = {"nonce": query.get("nonce", ""), "scope": query.get("scope", ""),
                            "client_id": query.get("client_id", "")}
        target = query["redirect_uri"] + "?" + urlencode({"code": code, "state": query["state"]})
        count("authorize")
        return self.reply(302, b"", {"Location": target})

    def issue(self, scope, refresh=None):
        access = "soak-access-" + secrets.token_urlsafe(18)
        S["access"][access] = now() + ACCESS_SECONDS
        reply = {"access_token": access, "token_type": "Bearer", "expires_in": ACCESS_SECONDS,
                 "scope": scope}
        if refresh:
            reply["refresh_token"] = refresh
        return reply

    def token(self):
        body = self.form()
        if fault() in ("error500", "hang"):
            count("token_500")
            return self.reply(500, {"error": "server_error"})
        if body.get("grant_type") == "authorization_code":
            grant = S["codes"].pop(body.get("code", ""), None)
            if not grant:
                return self.reply(400, {"error": "invalid_grant"})
            refresh = "soak-refresh-" + secrets.token_urlsafe(18)
            S["refresh"][refresh] = grant["scope"]
            reply = self.issue(grant["scope"], refresh)
            t = int(now())
            reply["id_token"] = sign({
                "iss": "https://accounts.google.com", "sub": "soak-owner-1", "aud": grant["client_id"],
                "azp": grant["client_id"], "exp": t + 3600, "iat": t, "nonce": grant["nonce"],
                "email": "owner@example.test", "email_verified": True,
            })
            count("token_code")
            return self.reply(200, reply)
        if body.get("grant_type") == "refresh_token":
            scope = S["refresh"].get(body.get("refresh_token", ""))
            if scope is None:
                count("token_invalid_grant")
                return self.reply(400, {"error": "invalid_grant"})
            count("token_refresh")
            return self.reply(200, self.issue(scope))
        return self.reply(400, {"error": "unsupported_grant_type"})

    def gmail(self, method, path, query):
        rest = path.removeprefix("/gmail/v1/users/me/")
        if rest == "profile":
            return self.reply(200, {"emailAddress": "owner@example.test", "historyId": str(S["history_id"])})
        if rest == "labels":
            return self.reply(200, {"labels": [{"id": "INBOX", "name": "INBOX", "type": "system"}]})
        if rest == "messages" and method == "GET":
            limit = int(query.get("maxResults", "20"))
            listed = [{"id": m["id"], "threadId": m["threadId"]} for m in S["messages"][:limit]]
            reply = {"resultSizeEstimate": len(listed)}
            if listed:
                reply["messages"] = listed
            count("gmail_list")
            return self.reply(200, reply)
        if rest.startswith("messages/") and method == "GET":
            message_id = rest.split("/", 1)[1]
            for message in S["messages"]:
                if message["id"] == message_id:
                    count("gmail_get")
                    if message_id not in S.setdefault("fetched", []):
                        S["fetched"].append(message_id)
                        del S["fetched"][:-2000]
                    return self.reply(200, gmail_message(message))
            return self.reply(404, {"error": {"code": 404}})
        if rest == "history":
            start = int(query.get("startHistoryId", "0"))
            count("gmail_history")
            if start < S["min_history"]:
                count("served_history_404")
                return self.reply(404, {"error": {"code": 404, "message": "historyId too old"}})
            offset = int(query.get("pageToken", "0") or 0)
            limit = int(query.get("maxResults", "100"))
            newer = [h for h in S["history"] if h["id"] > start]
            page = newer[offset:offset + limit]
            reply = {"history": [{"id": str(h["id"]), "messagesAdded": [
                {"message": {"id": h["message"], "threadId": "t" + h["message"][1:], "labelIds": ["INBOX"]}}]}
                for h in page]}
            if offset + limit < len(newer):
                reply["nextPageToken"] = str(offset + limit)
            else:
                reply["historyId"] = str(S["history_id"])
            return self.reply(200, reply)
        return self.reply(404, {"error": {"code": 404}})

    def calendar(self, path, query):
        if path.endswith("/calendarList"):
            return self.reply(200, {"items": [{"id": "primary", "summary": "Owner", "primary": True,
                                               "accessRole": "owner"}]})
        if not path.endswith("/events"):
            return self.reply(404, {"error": {"code": 404}})
        start, end = parse_iso(query["timeMin"]), parse_iso(query["timeMax"])
        limit = int(query.get("maxResults", "50"))
        offset = int(query.get("pageToken", "0") or 0)
        matching = sorted((e for e in S["events"] if e["end"] > start and e["start"] < end),
                          key=lambda e: e["start"])
        page = matching[offset:offset + limit]
        count("calendar_list")
        reply = {"items": [{
            "id": e["id"], "status": "confirmed", "summary": e["summary"], "etag": f'"{e["id"]}"',
            "eventType": "default", "visibility": "default",
            "start": {"dateTime": iso(e["start"])}, "end": {"dateTime": iso(e["end"])},
        } for e in page]}
        if offset + limit < len(matching):
            reply["nextPageToken"] = str(offset + limit)
        return self.reply(200, reply)


def main():
    global S, JWK
    os.makedirs(HOME, exist_ok=True)
    S = load()
    JWK = signing_key()
    server = ThreadingHTTPServer(("127.0.0.1", PORT), Handler)
    print(f"fake provider on 127.0.0.1:{PORT}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
