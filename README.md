# JsonStub

JsonStub is a tiny Rust server that exposes files in `json/` as HTTP endpoints.
It is meant to quickly mock API responses from local JSON files.

## Features

- Serves any file under `json/` at `/json/<subdir>/<path>`
- Auto-reloads on file changes (reads from disk on every request)
- File system watch with event logs (create/modify/delete)
- Web UI with endpoint list and per-folder upload
- Folder creation from the UI

## Quick start

```bash
cargo run
```

Open:

- `http://127.0.0.1:3000/` or `http://127.0.0.1:3000/json`

## Folder layout

```
json/
  ipv4/
    file.json
  ipv6/
    file.json
  fqdn/
    file.json
```

Each file becomes reachable at:

```
http://127.0.0.1:3000/json/<subdir>/<file>
```

Examples:

- `http://127.0.0.1:3000/json/ipv4/file.json`
- `http://127.0.0.1:3000/json/ipv6/file.json`
- `http://127.0.0.1:3000/json/fqdn/file.json`

## UI

- Home list: `/` or `/json`
- Folder view + upload: `/json/<subdir>`
- Create folder: form on `/json`

Uploads keep the original file name and are saved under `json/<subdir>/`.

## Logging

Requests and filesystem events are logged to stdout.

## Notes

- Responses are served with `Cache-Control: no-store`
- Only safe path segments are allowed to avoid traversal

## License

MIT
