   set -e
   tools="$HOME/.local/share/pi-desktop-build"
   npm install --prefix "$tools" --ignore-scripts bun@1.4.2
   (cd "$tools/node_modules/bun" && node install.js)

   export PATH="$tools/node_modules/.bin:$HOME/.cargo/bin:$PATH"

   (cd backend/durable &&
    npm ci --ignore-scripts &&
    bun run build)

   PI_DESKTOP_DURABLE_BINARY="$PWD/artifacts/durable/pi-desktop-durable" \
     cargo build --locked --release -p pi_remote --features bundled-durable
