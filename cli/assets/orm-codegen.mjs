import { writeFileSync } from 'node:fs';
import { createClient, Auth } from '@kalamdb/client';
import { generateSchema } from '@kalamdb/orm';

const url = process.env.KALAM_SCHEMA_URL;
const namespace = process.env.KALAM_SCHEMA_NAMESPACE;
const out = process.env.KALAM_SCHEMA_OUT;
const authMode = process.env.KALAM_SCHEMA_AUTH_MODE;

const authProvider = authMode === 'basic'
  ? async () => Auth.basic(process.env.KALAM_SCHEMA_USER ?? 'root', process.env.KALAM_SCHEMA_PASSWORD ?? '')
  : authMode === 'jwt'
    ? async () => Auth.jwt(process.env.KALAM_SCHEMA_TOKEN ?? '')
    : undefined;

const client = createClient(authProvider ? { url, authProvider } : { url });
if (typeof client.initialize === 'function') {
  await client.initialize();
}

const schema = await generateSchema(client, {
  ...(namespace ? { namespaces: [namespace] } : {}),
  includeSystemColumns: true,
});
writeFileSync(out, schema);

if (typeof client.disconnect === 'function') {
  await client.disconnect();
}
