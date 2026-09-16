# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: functions-sql-studio.spec.ts >> SQL Studio deploys a typed booking function and Test UI can invoke it
- Location: tests/e2e/functions-sql-studio.spec.ts:147:1

# Error details

```
Error: expect(locator).toContainText(expected) failed

Locator: getByTestId('function-inline-source').locator('.view-line')
Expected substring: "venue_id"
Error: strict mode violation: getByTestId('function-inline-source').locator('.view-line') resolved to 21 elements:
    1) <div class="view-line">…</div> aka locator('.view-line').first()
    2) <div class="view-line">…</div> aka locator('.view-lines > div:nth-child(2)')
    3) <div class="view-line">…</div> aka locator('.view-lines > div:nth-child(3)')
    4) <div class="view-line">…</div> aka locator('.view-lines > div:nth-child(4)')
    5) <div class="view-line">…</div> aka locator('.view-lines > div:nth-child(5)')
    6) <div class="view-line">…</div> aka locator('.view-lines > div:nth-child(6)')
    7) <div class="view-line">…</div> aka locator('.view-lines > div:nth-child(7)')
    8) <div class="view-line">…</div> aka locator('.view-lines > div:nth-child(8)')
    9) <div class="view-line">…</div> aka locator('.view-lines > div:nth-child(9)')
    10) <div class="view-line">…</div> aka locator('.view-lines > div:nth-child(10)')
    ...

Call log:
  - Expect "toContainText" getByTestId('function-inline-source').locator('.view-line') with timeout 5000ms
  - waiting for getByTestId('function-inline-source').locator('.view-line')

```

# Page snapshot

```yaml
- generic [ref=e1]:
  - generic [ref=e2]:
    - generic [ref=e5]:
      - banner [ref=e6]:
        - generic [ref=e7]:
          - img "KalamDB" [ref=e8]
          - button "KalamDB Admin" [ref=e9]
          - button "Connect" [ref=e10]
        - generic [ref=e12]:
          - textbox "Search..." [ref=e13]
          - generic: Cmd K
        - generic [ref=e14]:
          - generic "WS Online" [ref=e15]:
            - generic [ref=e17]: WS
            - generic [ref=e18]: Online
            - generic [ref=e19]: 127.0.0.1:2900
          - button "Open notifications" [ref=e24]
          - button "RO" [ref=e25]
      - generic [ref=e27]:
        - complementary [ref=e28]:
          - navigation "Primary navigation" [ref=e29]:
            - link [ref=e30] [cursor=pointer]:
              - /url: /ui/dashboard
            - link [ref=e36] [cursor=pointer]:
              - /url: /ui/sql
            - link [ref=e39] [cursor=pointer]:
              - /url: /ui/schedules
            - link [ref=e43] [cursor=pointer]:
              - /url: /ui/functions
            - link [ref=e47] [cursor=pointer]:
              - /url: /ui/streaming/topics
            - link [ref=e55] [cursor=pointer]:
              - /url: /ui/users
            - link [ref=e61] [cursor=pointer]:
              - /url: /ui/live-queries
            - link [ref=e66] [cursor=pointer]:
              - /url: /ui/logging
          - generic [ref=e70]:
            - link [ref=e72] [cursor=pointer]:
              - /url: /ui/settings
            - button "Expand sidebar" [ref=e76]
        - main [ref=e77]:
          - generic [ref=e80]:
            - generic [ref=e83]:
              - generic [ref=e84]:
                - button "Explorer" [ref=e85]
                - button "Editor" [ref=e90]
              - generic [ref=e96]:
                - generic [ref=e97]:
                  - generic [ref=e98]:
                    - generic [ref=e99]: Namespace
                    - button "Refresh explorer" [ref=e101]
                  - combobox [ref=e102]:
                    - generic: blog
                  - textbox "Filter tables..." [ref=e104]
                - generic [ref=e105]:
                  - button "Favorites 0" [ref=e106]:
                    - generic [ref=e112]: Favorites
                    - generic [ref=e113]: "0"
                  - paragraph [ref=e115]: No saved queries yet.
                - generic [ref=e116]: Tables (2)
                - list [ref=e121]:
                  - listitem [ref=e122]:
                    - generic [ref=e123]:
                      - button "Collapse blogs" [ref=e124]
                      - generic [ref=e133]: blogs
                      - generic [ref=e134]: "7"
                    - generic [ref=e135]:
                      - generic [ref=e136]:
                        - generic [ref=e141]: blog_id
                        - generic [ref=e142]: BigInt
                      - generic [ref=e143]:
                        - generic [ref=e147]: content
                        - generic [ref=e148]: Text
                      - generic [ref=e149]:
                        - generic [ref=e153]: summary
                        - generic [ref=e154]: Text
                      - generic [ref=e155]:
                        - generic [ref=e159]: created
                        - generic [ref=e160]: Timestamp
                      - generic [ref=e161]:
                        - generic [ref=e165]: updated
                        - generic [ref=e166]: Timestamp
                      - generic [ref=e167]:
                        - generic [ref=e171]: _seq
                        - generic [ref=e172]: BigInt
                      - generic [ref=e173]:
                        - generic [ref=e177]: _deleted
                        - generic [ref=e178]: Boolean
                  - listitem [ref=e179]:
                    - generic [ref=e180]:
                      - button "Expand summary_failures" [ref=e181]
                      - generic [ref=e190]: summary_failures
                      - generic [ref=e191]: "7"
            - separator [ref=e192]
            - generic [ref=e196]:
              - button "Expand details panel" [ref=e198]
              - generic [ref=e200]:
                - button "Untitled query Draft" [ref=e201]:
                  - generic [ref=e203]:
                    - generic [ref=e209]: Untitled query
                    - generic [ref=e210]: Draft
                - button "New query tab" [ref=e211]
              - generic [ref=e213]:
                - code [ref=e220]:
                  - generic [ref=e221]:
                    - textbox "Editor content" [ref=e222]
                    - textbox [aria-hidden] [ref=e223]
                    - generic [aria-hidden] [ref=e225]:
                      - generic [ref=e226]: "1"
                      - generic [ref=e228]: "2"
                      - generic [ref=e230]: "3"
                      - generic [ref=e232]: "4"
                    - generic [aria-hidden] [ref=e241]:
                      - generic [ref=e242]: SELECT name, value
                      - generic [ref=e244]: FROM system.settings
                      - generic [ref=e246]: WHERE name IN ('cluster.cluster_id', 'server.host', 'topics.visibility_timeout_secs')
                      - generic [ref=e248]: ORDER BY name;
                - separator [ref=e251]
                - generic [ref=e255]:
                  - generic [ref=e256]:
                    - tablist [ref=e258]:
                      - tab "Results" [active] [selected] [ref=e259]
                      - tab "Log (1)" [ref=e261]
                    - generic [ref=e264]:
                      - generic [ref=e265]:
                        - generic [ref=e266]: 3 rows
                        - generic [ref=e267]: took 1 ms
                        - generic [ref=e268]: "as user: root"
                      - generic [ref=e269]:
                        - generic [ref=e270]:
                          - generic [ref=e271]:
                            - switch [ref=e272]
                            - generic [ref=e273]: Live
                          - generic [ref=e281]: Never saved
                        - button "Save" [ref=e285]
                        - generic [ref=e286]:
                          - button "Run" [ref=e287]
                          - button "Execute options" [ref=e288]
                        - button "More query actions" [ref=e289]
                  - table [ref=e294]:
                    - rowgroup [ref=e299]:
                      - row [ref=e300]:
                        - columnheader [ref=e301]
                        - columnheader [ref=e303]:
                          - button "name Text" [ref=e304]:
                            - generic [ref=e305]: name
                            - generic [ref=e306]: Text
                          - button "Resize name" [ref=e310]
                        - columnheader [ref=e311]:
                          - button "value Text" [ref=e312]:
                            - generic [ref=e313]: value
                            - generic [ref=e314]: Text
                          - button "Resize value" [ref=e318]
                    - rowgroup [ref=e319]:
                      - row [ref=e320]:
                        - cell [ref=e321]
                        - cell "cluster.cluster_id" [ref=e323]
                        - cell "cluster" [ref=e326]
                      - row [ref=e329]:
                        - cell [ref=e330]
                        - cell "server.host" [ref=e332]
                        - cell "0.0.0.0" [ref=e335]
                      - row [ref=e338]:
                        - cell [ref=e339]
                        - cell "topics.visibility_timeout_secs" [ref=e341]
                        - cell "30" [ref=e344]
                  - generic [ref=e348]:
                    - button [disabled]
                    - generic [ref=e349]: Page
                    - spinbutton "Results page" [ref=e350]: "1"
                    - generic [ref=e351]: of 1
                    - button [disabled]
                    - combobox [ref=e352]:
                      - generic: 100 rows
                    - generic [ref=e353]: 3 records
    - region "Notifications alt+T"
  - generic [aria-hidden] [ref=e354]: "25"
  - generic [ref=e355]:
    - alert
    - alert
```

# Test source

```ts
  137 | `.trim();
  138 | }
  139 | 
  140 | async function openFunctionsList(page: Page, search: string): Promise<void> {
  141 |   await clickNav(page, "/ui/functions");
  142 |   await expect(page.getByPlaceholder("Search functions...")).toBeVisible({ timeout: 30_000 });
  143 |   await refreshProcedureCatalog(page);
  144 |   await page.getByPlaceholder("Search functions...").fill(search);
  145 | }
  146 | 
  147 | test("SQL Studio deploys a typed booking function and Test UI can invoke it", async ({ page }) => {
  148 |   await skipUnlessLiveAdminUi(page);
  149 | 
  150 |   const ns = uniqueNamespace();
  151 |   const procedureId = `${ns}.book_tickets`;
  152 | 
  153 |   try {
  154 |     await openSqlStudio(page);
  155 | 
  156 |     await test.step("create enum, table row type, request object, and deploy procedure", async () => {
  157 |       await runSql(page, deploySql(ns));
  158 |       await page.getByRole("tab", { name: /^Log/ }).click();
  159 |       await expect(page.getByText(/created|replaced|INSERT/i).first()).toBeVisible({ timeout: 10_000 });
  160 |     });
  161 | 
  162 |     await test.step("system.procedures lists implementation, signature, and inline source", async () => {
  163 |       await openSqlStudio(page);
  164 |       await runSql(page, catalogSql(ns));
  165 |       const rows = await readSqlRows(
  166 |         page,
  167 |         [
  168 |           "procedure_id",
  169 |           "name",
  170 |           "signature",
  171 |           "return_type",
  172 |           "implementation",
  173 |           "revision_id",
  174 |           "security",
  175 |           "comment",
  176 |           "language",
  177 |           "source",
  178 |         ],
  179 |         2,
  180 |       );
  181 |       const later = rows.find((row) => row.name === "later");
  182 |       const booking = rows.find((row) => row.name === "book_tickets");
  183 |       expect(later?.implementation).toBe("missing");
  184 |       expect(later?.comment).toContain("Not implemented yet");
  185 |       expect(["", "null", "NULL", "\u2014"]).toContain((later?.source ?? "").trim());
  186 |       expect(booking?.implementation).toBe("inline");
  187 |       expect(booking?.signature).toContain("venue_id");
  188 |       expect(booking?.return_type).toContain("booking");
  189 |       expect(booking?.comment).toContain("Playwright concert booking");
  190 |       expect(booking?.language.toLowerCase()).toContain("javascript");
  191 |       expect(booking?.revision_id).toMatch(/^inline:/);
  192 |       expect(booking?.source).toContain("venue_id");
  193 |       expect(booking?.security.toLowerCase()).toContain("invoker");
  194 |     });
  195 | 
  196 |     await test.step("CALL the procedure from SQL Studio", async () => {
  197 |       await runSql(page, callSql(ns));
  198 |       await page.getByRole("tab", { name: /^Results/ }).click();
  199 |       await expect(page.getByTestId("results-column-header-result")).toBeVisible({ timeout: 15_000 });
  200 |       await page.locator('[data-column-name="result"]').first().dblclick();
  201 |       await expect(page.getByRole("dialog")).toContainText(/BK-HALL-A-42|Cairo|confirmed|Ada Lovelace/i, {
  202 |         timeout: 10_000,
  203 |       });
  204 |       await page.getByRole("button", { name: "Cancel" }).click();
  205 |     });
  206 | 
  207 |     await test.step("Functions list shows status and column values", async () => {
  208 |       await openFunctionsList(page, procedureId);
  209 |       await expect(page.getByRole("columnheader", { name: "Function" })).toBeVisible();
  210 |       await expect(page.getByRole("columnheader", { name: "Status" })).toBeVisible();
  211 |       await expect(page.getByRole("columnheader", { name: "Current revision" })).toBeVisible();
  212 |       await expect(page.getByRole("columnheader", { name: "Last updated" })).toBeVisible();
  213 |       await expect(page.getByRole("columnheader", { name: "Calls 24h" })).toBeVisible();
  214 | 
  215 |       const bookingRow = page.getByTestId(`functions-row-${procedureId}`);
  216 |       await expect(bookingRow).toBeVisible({ timeout: 30_000 });
  217 |       await expect(bookingRow).toContainText("Ready");
  218 |       await expect(bookingRow).toContainText("Playwright concert booking");
  219 |       await expect(bookingRow).toContainText("inline:");
  220 |       await expect(bookingRow.locator("td").nth(4)).toHaveText(/^\d/);
  221 | 
  222 |       await page.getByPlaceholder("Search functions...").fill(`${ns}.later`);
  223 |       const laterRow = page.getByTestId(`functions-row-${ns}.later`);
  224 |       await expect(laterRow).toBeVisible({ timeout: 15_000 });
  225 |       await expect(laterRow).toContainText("Unimplemented");
  226 |       await expect(laterRow).toContainText("Not implemented yet");
  227 |     });
  228 | 
  229 |     await test.step("function overview shows inline source", async () => {
  230 |       await openFunctionsList(page, procedureId);
  231 |       await page.getByTestId(`functions-row-${procedureId}`).click();
  232 |       await expect(page.getByRole("tab", { name: "Overview" })).toHaveAttribute("data-state", "active");
  233 |       await expect(page.getByText("Inline script")).toBeVisible();
  234 |       const inlineSource = page.getByTestId("function-inline-source");
  235 |       await expect(inlineSource).toBeVisible();
  236 |       await expect(inlineSource.locator(".monaco-editor")).toBeVisible({ timeout: 30_000 });
> 237 |       await expect(inlineSource.locator(".view-line")).toContainText("venue_id");
      |                                                        ^ Error: expect(locator).toContainText(expected) failed
  238 |       await expect(page.getByTestId("function-runtime-memory")).toBeVisible();
  239 |     });
  240 | 
  241 |     await test.step("invoke the same procedure from the Functions Test UI", async () => {
  242 |       await openFunctionTest(page, procedureId);
  243 | 
  244 |       await fillLabeledInput(page, "venue_id", "hall-a");
  245 |       await fillLabeledInput(page, "display_name", "Ada Lovelace");
  246 |       await fillLabeledInput(page, "seats", "2");
  247 |       await fillLabeledInput(page, "notes", "aisle");
  248 | 
  249 |       const vip = page.getByRole("switch", { name: "vip" });
  250 |       await expect(vip).toBeVisible();
  251 |       if ((await vip.getAttribute("data-state")) !== "checked") {
  252 |         await vip.click();
  253 |       }
  254 | 
  255 |       await page.getByRole("combobox", { name: "status" }).click();
  256 |       await page.getByRole("option", { name: "confirmed" }).click();
  257 | 
  258 |       await page.getByRole("button", { name: "Run test" }).click();
  259 |       await expect(page.getByText("Ada Lovelace")).toBeVisible({ timeout: 30_000 });
  260 |       await expect(page.getByText(/BK-HALL-A-42/)).toBeVisible();
  261 |       await expect(page.getByText("Cairo")).toBeVisible();
  262 |       await expect(page.getByText('"confirmed"', { exact: true })).toBeVisible();
  263 |     });
  264 | 
  265 |     await test.step("overview shows isolate memory after a successful invoke", async () => {
  266 |       await page.getByRole("tab", { name: "Overview" }).click();
  267 |       const memory = page.getByTestId("function-runtime-memory");
  268 |       await expect(memory).toBeVisible();
  269 |       await expect(memory.getByText("Used heap", { exact: true })).toBeVisible({
  270 |         timeout: 15_000,
  271 |       });
  272 |       await expect(memory.getByText("Peak heap", { exact: true })).toBeVisible();
  273 |       await expect(memory.getByText("Reserved", { exact: true })).toBeVisible();
  274 |     });
  275 |   } finally {
  276 |     await openSqlStudio(page).catch(() => undefined);
  277 |     await runSql(page, `DROP NAMESPACE IF EXISTS ${ns} CASCADE;`).catch(() => undefined);
  278 |   }
  279 | });
  280 | 
```