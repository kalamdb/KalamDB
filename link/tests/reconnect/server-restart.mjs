import { createClient, Auth } from '../../sdks/typescript/client/dist/src/index.js';
import { writeFile, access } from 'node:fs/promises';
const url=process.env.KALAMDB_URL;
const directory=process.env.KALAM_RESTART_COORD_DIR;
const client=createClient({url,authProvider: async()=>Auth.basic(process.env.KALAMDB_USER,process.env.KALAMDB_PASSWORD)});
const writer=createClient({url,authProvider: async()=>Auth.basic(process.env.KALAMDB_USER,process.env.KALAMDB_PASSWORD)});
const wait=async f=>{for(let i=0;i<2400;i++){if(await f())return;await new Promise(r=>setTimeout(r,50))}throw Error('timed out')};
const events=[];
try {
  await writer.query('CREATE SCHEMA IF NOT EXISTS ts_restart');
  await writer.query('CREATE TABLE IF NOT EXISTS ts_restart.items (id INT PRIMARY KEY, payload TEXT)');
  const unsubscribe=await client.liveEvents('SELECT * FROM ts_restart.items',e=>events.push(e));
  const has=id=>events.some(e=>e.rows?.some(r=>Number(r.id.toJson())===id));

  await writer.query("INSERT INTO ts_restart.items (id, payload) VALUES (1, 'before')");
  await wait(()=>has(1));
  await writeFile(`${directory}/ts-ready`,'ready');
  await wait(async()=>access(`${directory}/restarted`).then(()=>true,()=>false));
  await wait(()=>events.filter(e=>e.type==='subscription_ack').length>=2);
  await writer.query("INSERT INTO ts_restart.items (id, payload) VALUES (2, 'after')");
  await wait(()=>has(2));
  console.log('TypeScript reconnected and received post-restart insert');
  await unsubscribe();
} finally { await client.shutdown(); await writer.shutdown(); }
