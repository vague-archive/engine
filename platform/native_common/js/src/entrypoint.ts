import type { Engine } from '@vaguevoid/engine'

/**
 * This file is the entrypoint for JS based modules.
 * This file is embedded into the engine during build.
 */

class NativeEngine implements Engine {
  // Memory Related
  readonly POINTER_SIZE = 8
  readonly getArrayBuffer = Deno.UnsafePointerView.getArrayBuffer as unknown as Engine['getArrayBuffer']
  readonly getString = Deno.UnsafePointerView.getCString as unknown as Engine['getString']
  readonly debugAddress = Extension.debugAddress
  readonly nullPointer = Extension.nullPointer as Engine['nullPointer']
  readonly malloc = Extension.malloc
  readonly free = Extension.free
  readonly getUint8 = Extension.getUint8
  readonly getInt8 = Extension.getInt8
  readonly getUint16 = Extension.getUint16
  readonly getInt16 = Extension.getInt16
  readonly getUint32 = Extension.getUint32
  readonly getInt32 = Extension.getInt32
  readonly getBigUint64 = Extension.getBigUint64
  readonly getBigInt64 = Extension.getBigInt64
  readonly getFloat32 = Extension.getFloat32
  readonly getFloat64 = Extension.getFloat64
  readonly getPointer = Extension.getPointer
  readonly setUint8 = Extension.setUint8
  readonly setInt8 = Extension.setInt8
  readonly setUint16 = Extension.setUint16
  readonly setInt16 = Extension.setInt16
  readonly setUint32 = Extension.setUint32
  readonly setInt32 = Extension.setInt32
  readonly setBigUint64 = Extension.setBigUint64
  readonly setBigInt64 = Extension.setBigInt64
  readonly setFloat32 = Extension.setFloat32
  readonly setFloat64 = Extension.setFloat64
  readonly setPointer = Extension.setPointer

  // Engine API's
  readonly spawn = Extension.spawn
  readonly despawn = Extension.despawn
  readonly loadScene = Extension.loadScene
  readonly inputBufferPointer = Extension.inputBufferPointer
  readonly inputBufferLength = Extension.inputBufferLength
  readonly addComponents = Extension.addComponents
  readonly removeComponents = Extension.removeComponents
  readonly getEntityLabel = Extension.getEntityLabel
  readonly setEntityLabel = Extension.setEntityLabel
  readonly queryLen = Extension.queryLen
  readonly queryGet = Extension.queryGet
  readonly queryGetEntity = Extension.queryGetEntity
  readonly queryGetLabel = Extension.queryGetLabel
  readonly setSystemEnabled = Extension.setSystemEnabled
  readonly setParent = Extension.setParent
  readonly clearParent = Extension.clearParent
  readonly getParent = Extension.getParent
  readonly eventCount = Extension.eventCount
  readonly eventGet = Extension.eventGet
  readonly eventSend = Extension.eventSend

  // Texture Asset Manager
  readonly gpuInterfaceGetTextureAssetManagerMut = Extension.gpuInterfaceGetTextureAssetManagerMut
  readonly textureAssetManagerWhiteTextureId = Extension.textureAssetManagerWhiteTextureId
  readonly textureAssetManagerMissingTextureId = Extension.textureAssetManagerMissingTextureId
  readonly textureAssetManagerRegisterNextTextureId = Extension.textureAssetManagerRegisterNextTextureId
  readonly textureAssetManagerCreatePendingTexture = Extension.textureAssetManagerCreatePendingTexture
  readonly textureAssetManagerFreePendingTexture = Extension.textureAssetManagerFreePendingTexture
  readonly textureAssetManagerFreeEngineTexture = Extension.textureAssetManagerFreeEngineTexture
  readonly textureAssetManagerFreeLoadedTexture = Extension.textureAssetManagerFreeLoadedTexture
  readonly textureAssetManagerFreeFailedTexture = Extension.textureAssetManagerFreeFailedTexture
  readonly textureAssetManagerGetTextureTypeById = Extension.textureAssetManagerGetTextureTypeById
  readonly textureAssetManagerGetPendingTextureById = Extension.textureAssetManagerGetPendingTextureById
  readonly textureAssetManagerGetEngineTextureById = Extension.textureAssetManagerGetEngineTextureById
  readonly textureAssetManagerGetLoadedTextureById = Extension.textureAssetManagerGetLoadedTextureById
  readonly textureAssetManagerGetFailedTextureById = Extension.textureAssetManagerGetFailedTextureById
  readonly textureAssetManagerGetTextureTypeByPath = Extension.textureAssetManagerGetTextureTypeByPath
  readonly textureAssetManagerGetPendingTextureByPath = Extension.textureAssetManagerGetPendingTextureByPath
  readonly textureAssetManagerGetEngineTextureByPath = Extension.textureAssetManagerGetEngineTextureByPath
  readonly textureAssetManagerGetLoadedTextureByPath = Extension.textureAssetManagerGetLoadedTextureByPath
  readonly textureAssetManagerGetFailedTextureByPath = Extension.textureAssetManagerGetFailedTextureByPath
  readonly textureAssetManagerAreIdsLoaded = Extension.textureAssetManagerAreIdsLoaded
  readonly textureAssetManagerIsIdLoaded = Extension.textureAssetManagerIsIdLoaded
  readonly textureAssetManagerLoadTexture = Extension.textureAssetManagerLoadTexture
  readonly textureAssetManagerLoadTextureByPendingTexture = Extension.textureAssetManagerLoadTextureByPendingTexture

  // Text Asset Manager
  readonly textAssetManagerRegisterNextTextId = Extension.textAssetManagerRegisterNextTextId
  readonly textAssetManagerCreatePendingText = Extension.textAssetManagerCreatePendingText
  readonly textAssetManagerFreePendingText = Extension.textAssetManagerFreePendingText
  readonly textAssetManagerFreeEngineText = Extension.textAssetManagerFreeEngineText
  readonly textAssetManagerFreeLoadedText = Extension.textAssetManagerFreeLoadedText
  readonly textAssetManagerFreeFailedText = Extension.textAssetManagerFreeFailedText
  readonly textAssetManagerGetTextTypeById = Extension.textAssetManagerGetTextTypeById
  readonly textAssetManagerGetPendingTextById = Extension.textAssetManagerGetPendingTextById
  readonly textAssetManagerGetEngineTextById = Extension.textAssetManagerGetEngineTextById
  readonly textAssetManagerGetLoadedTextById = Extension.textAssetManagerGetLoadedTextById
  readonly textAssetManagerGetFailedTextById = Extension.textAssetManagerGetFailedTextById
  readonly textAssetManagerGetTextTypeByPath = Extension.textAssetManagerGetTextTypeByPath
  readonly textAssetManagerGetPendingTextByPath = Extension.textAssetManagerGetPendingTextByPath
  readonly textAssetManagerGetEngineTextByPath = Extension.textAssetManagerGetEngineTextByPath
  readonly textAssetManagerGetLoadedTextByPath = Extension.textAssetManagerGetLoadedTextByPath
  readonly textAssetManagerGetFailedTextByPath = Extension.textAssetManagerGetFailedTextByPath
  readonly textAssetManagerAreIdsLoaded = Extension.textAssetManagerAreIdsLoaded
  readonly textAssetManagerIsIdLoaded = Extension.textAssetManagerIsIdLoaded
  readonly textAssetManagerLoadText = Extension.textAssetManagerLoadText
  readonly textAssetManagerLoadTextByPendingText = Extension.textAssetManagerLoadTextByPendingText
}

Extension.registerEngine(new NativeEngine())
