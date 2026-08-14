/// <reference path="../pb_data/types.d.ts" />
migrate((app) => {
  const collection = app.findCollectionByNameOrId("pbc_669929365")

  // update collection data
  unmarshal({
    "createRule": "room = @collection.room_members.room && @request.auth.id = @collection.room_members.user"
  }, collection)

  return app.save(collection)
}, (app) => {
  const collection = app.findCollectionByNameOrId("pbc_669929365")

  // update collection data
  unmarshal({
    "createRule": null
  }, collection)

  return app.save(collection)
})
