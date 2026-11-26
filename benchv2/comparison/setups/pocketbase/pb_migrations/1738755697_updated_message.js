/// <reference path="../pb_data/types.d.ts" />
migrate((app) => {
  const collection = app.findCollectionByNameOrId("pbc_669929365")

  // Both comparison workloads use a signed-in user without a room-membership check.
  unmarshal({
    "createRule": "@request.auth.id != ''",
    "listRule": "@request.auth.id != ''",
    "viewRule": "@request.auth.id != ''"
  }, collection)

  return app.save(collection)
}, (app) => {
  const collection = app.findCollectionByNameOrId("pbc_669929365")

  // update collection data
  unmarshal({
    "createRule": null,
    "listRule": null,
    "viewRule": ""
  }, collection)

  return app.save(collection)
})
