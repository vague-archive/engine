import * as ops from 'ext:core/ops'

/**
 * This file is meant for *binding* Deno extensions to the global scope.
 *
 * Any Deno Extension registered via the WorkerOptions.extensions field
 * will be available from the "ext:core/ops" package.
 */

declare global {
  // deno-lint-ignore no-explicit-any
  var Extension: Record<string, (...args: any[]) => any>
}

globalThis.Extension = {
  // Memory Related
  debugAddress: ops.op_fiasco_debug_addr,
  nullPointer: ops.op_fiasco_null_ptr,
  malloc: ops.op_fiasco_alloc,
  free: ops.op_fiasco_dealloc,
  getUint8: ops.op_fiasco_read_u8,
  getInt8: ops.op_fiasco_read_i8,
  getUint16: ops.op_fiasco_read_u16,
  getInt16: ops.op_fiasco_read_i16,
  getUint32: ops.op_fiasco_read_u32,
  getInt32: ops.op_fiasco_read_i32,
  getBigUint64: ops.op_fiasco_read_u64,
  getBigInt64: ops.op_fiasco_read_i64,
  getFloat32: ops.op_fiasco_read_f32,
  getFloat64: ops.op_fiasco_read_f64,
  getPointer: ops.op_fiasco_read_ptr,
  setUint8: ops.op_fiasco_write_u8,
  setInt8: ops.op_fiasco_write_i8,
  setUint16: ops.op_fiasco_write_u16,
  setInt16: ops.op_fiasco_write_i16,
  setUint32: ops.op_fiasco_write_u32,
  setInt32: ops.op_fiasco_write_i32,
  setBigUint64: ops.op_fiasco_write_u64,
  setBigInt64: ops.op_fiasco_write_i64,
  setFloat32: ops.op_fiasco_write_f32,
  setFloat64: ops.op_fiasco_write_f64,
  setPointer: ops.op_fiasco_write_ptr,
  registerEngine: ops.op_fiasco_register_engine,
  getEngine: ops.op_fiasco_get_engine,

  // Engine API's
  spawn: ops.op_fiasco_spawn,
  despawn: ops.op_fiasco_despawn,
  loadScene: ops.op_fiasco_load_scene,
  inputBufferPointer: ops.op_fiasco_input_buffer_ptr,
  inputBufferLength: ops.op_fiasco_input_buffer_len,
  addComponents: ops.op_fiasco_add_components,
  removeComponents: ops.op_fiasco_remove_components,
  getEntityLabel: ops.op_fiasco_get_entity_label,
  setEntityLabel: ops.op_fiasco_set_entity_label,
  queryLen: ops.op_fiasco_query_len,
  queryGet: ops.op_fiasco_query_get,
  queryGetEntity: ops.op_fiasco_query_get_entity,
  queryGetLabel: ops.op_fiasco_query_get_label,
  setSystemEnabled: ops.op_fiasco_set_system_enabled,
  setParent: ops.op_fiasco_set_parent,
  clearParent: ops.op_fiasco_clear_parent,
  getParent: ops.op_fiasco_get_parent,
  eventCount: ops.op_fiasco_event_count,
  eventGet: ops.op_fiasco_event_get,
  eventSend: ops.op_fiasco_event_send,

  // Texture Asset Manager
  gpuInterfaceGetTextureAssetManagerMut: ops.op_gpu_interface_get_texture_asset_manager_mut,
  textureAssetManagerWhiteTextureId: ops.op_texture_asset_manager_white_texture_id,
  textureAssetManagerMissingTextureId: ops.op_texture_asset_manager_missing_texture_id,
  textureAssetManagerRegisterNextTextureId: ops.op_texture_asset_manager_register_next_texture_id,
  textureAssetManagerCreatePendingTexture: ops.op_texture_asset_manager_create_pending_texture,
  textureAssetManagerFreePendingTexture: ops.op_texture_asset_manager_free_pending_texture,
  textureAssetManagerFreeEngineTexture: ops.op_texture_asset_manager_free_engine_texture,
  textureAssetManagerFreeLoadedTexture: ops.op_texture_asset_manager_free_loaded_texture,
  textureAssetManagerFreeFailedTexture: ops.op_texture_asset_manager_free_failed_texture,
  textureAssetManagerGetTextureTypeById: ops.op_texture_asset_manager_get_texture_type_by_id,
  textureAssetManagerGetPendingTextureById: ops.op_texture_asset_manager_get_pending_texture_by_id,
  textureAssetManagerGetEngineTextureById: ops.op_texture_asset_manager_get_engine_texture_by_id,
  textureAssetManagerGetLoadedTextureById: ops.op_texture_asset_manager_get_loaded_texture_by_id,
  textureAssetManagerGetFailedTextureById: ops.op_texture_asset_manager_get_failed_texture_by_id,
  textureAssetManagerGetTextureTypeByPath: ops.op_texture_asset_manager_get_texture_type_by_path,
  textureAssetManagerGetPendingTextureByPath: ops.op_texture_asset_manager_get_pending_texture_by_path,
  textureAssetManagerGetEngineTextureByPath: ops.op_texture_asset_manager_get_engine_texture_by_path,
  textureAssetManagerGetLoadedTextureByPath: ops.op_texture_asset_manager_get_loaded_texture_by_path,
  textureAssetManagerGetFailedTextureByPath: ops.op_texture_asset_manager_get_failed_texture_by_path,
  textureAssetManagerAreIdsLoaded: ops.op_texture_asset_manager_are_ids_loaded,
  textureAssetManagerIsIdLoaded: ops.op_texture_asset_manager_is_id_loaded,
  textureAssetManagerLoadTexture: ops.op_texture_asset_manager_load_texture,
  textureAssetManagerLoadTextureByPendingTexture: ops.op_texture_asset_manager_load_texture_by_pending_texture,

  // Text Asset Manager
  textAssetManagerRegisterNextTextId: ops.op_text_asset_manager_register_next_text_id,
  textAssetManagerCreatePendingText: ops.op_text_asset_manager_create_pending_text,
  textAssetManagerFreePendingText: ops.op_text_asset_manager_free_pending_text,
  textAssetManagerFreeEngineText: ops.op_text_asset_manager_free_engine_text,
  textAssetManagerFreeLoadedText: ops.op_text_asset_manager_free_loaded_text,
  textAssetManagerFreeFailedText: ops.op_text_asset_manager_free_failed_text,
  textAssetManagerGetTextTypeById: ops.op_text_asset_manager_get_text_type_by_id,
  textAssetManagerGetPendingTextById: ops.op_text_asset_manager_get_pending_text_by_id,
  textAssetManagerGetEngineTextById: ops.op_text_asset_manager_get_engine_text_by_id,
  textAssetManagerGetLoadedTextById: ops.op_text_asset_manager_get_loaded_text_by_id,
  textAssetManagerGetFailedTextById: ops.op_text_asset_manager_get_failed_text_by_id,
  textAssetManagerGetTextTypeByPath: ops.op_text_asset_manager_get_text_type_by_path,
  textAssetManagerGetPendingTextByPath: ops.op_text_asset_manager_get_pending_text_by_path,
  textAssetManagerGetEngineTextByPath: ops.op_text_asset_manager_get_engine_text_by_path,
  textAssetManagerGetLoadedTextByPath: ops.op_text_asset_manager_get_loaded_text_by_path,
  textAssetManagerGetFailedTextByPath: ops.op_text_asset_manager_get_failed_text_by_path,
  textAssetManagerAreIdsLoaded: ops.op_text_asset_manager_are_ids_loaded,
  textAssetManagerIsIdLoaded: ops.op_text_asset_manager_is_id_loaded,
  textAssetManagerLoadText: ops.op_text_asset_manager_load_text,
  textAssetManagerLoadTextByPendingText: ops.op_text_asset_manager_load_text_by_pending_text,
}
