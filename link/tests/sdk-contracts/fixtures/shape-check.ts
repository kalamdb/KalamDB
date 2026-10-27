import { inventory, type Inventory } from './schema.js';
import { eq } from 'drizzle-orm';
const row: Inventory = {} as typeof inventory.$inferSelect;
const inferred: typeof inventory.$inferSelect = row;
eq(inventory.sku, 'sku-1');
// @ts-expect-error generated key is a string, not a number
eq(inventory.sku, 42);
// @ts-expect-error array elements retain their declared type
const invalid: typeof inventory.$inferInsert = {id: '1', sku: 's', tags: [42]};
void inferred;
void invalid;
