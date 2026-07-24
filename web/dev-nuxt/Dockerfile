# Build the Nuxt app, then run the Nitro server (needed for GitHub OAuth
# + the API routes). Set NUXT_SESSION_PASSWORD, NUXT_OAUTH_GITHUB_CLIENT_ID,
# NUXT_OAUTH_GITHUB_CLIENT_SECRET via `bp env`.
FROM node:22-alpine AS build
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci
COPY . .
RUN npm run build

FROM node:22-alpine
WORKDIR /app
COPY --from=build /app/.output ./.output
ENV HOST=0.0.0.0 PORT=3000 NODE_ENV=production
EXPOSE 3000
CMD ["node", ".output/server/index.mjs"]
