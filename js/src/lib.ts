// Events are generated in CI.
export * as Event from "./events/events"

/**
 * @private
 */
export type SizeAndAlignment = {
  /**
   * Not available at runtime
   */
  __size: number
  /**
   * Not available at runtime
   */
  __alignment: number
}

/**
 * Wrap types provided by the C API. Adds compile-time checks for correct size and alignment
 * when calling functions such as `malloc`/`free`.
 *
 * **Warning: Fields `__size` and `__alignment` are not available at runtime.**
 *
 * @private
 */
export type FFI<T extends SizeAndAlignment> = T

function makeApiVersion(major: number, minor: number, patch: number) {
  return (major << 25) | (minor << 15) | patch
}

export function voidTargetVersion() {
  return makeApiVersion(0, 0, 20)
}

export enum EcsType {
  AsyncCompletion,
  Component,
  Resource,
}

export enum ArgType {
  Completion,
  DataAccessMut,
  DataAccessRef,
  EventReader,
  EventWriter,
  Query,
}

export enum CreatePendingTexture {
  Success,
  OutputPendingTextureNull,
}

export enum GetTextureTypeByIdStatus {
  Success,
  TextureAssetManagerNull,
  TextureTypeNull,
  TextureIdNotFound,
}

export enum GetTextureByIdStatus {
  Success,
  TextureAssetManagerNull,
  OutputTextureNull,
  TextureIdNotFound,
  TextureTypeIncorrect,
}

export enum GetTextureTypeByPathStatus {
  Success,
  TexturePathNull,
  TextureAssetManagerNull,
  TextureTypeNull,
  TexturePathNotFound,
}

export enum GetTextureByPathStatus {
  Success,
  TexturePathNull,
  TextureAssetManagerNull,
  OutputTextureNull,
  TextureIdNotFound,
  TextureTypeIncorrect,
}

export enum LoadTextureStatus {
  Success,
  TextureAssetManagerNull,
  OutputPendingTextureNull,
  LoadTextureError,
}

export enum LoadTextureByPendingTextureStatus {
  Success,
  PendingTextureNull,
  TextureAssetManagerNull,
  LoadTextureError,
}

export type TextureHash = [number, number, number, number, number, number, number, number]

export enum TextureType {
  Pending,
  Engine,
  Loaded,
  Failed,
}

export type TextureFormat = "jpeg" | "png" | "unimplemented"

export type PendingTexture = FFI<{
  __size: 16
  __alignment: 8
  texturePath: string
  id: TextureId
  inAtlas: boolean
}>

export type LoadedTexture = FFI<{
  __size: 40
  __alignment: 8
  version: TextureHash
  texturePath: string
  formatType: TextureFormat
  id: TextureId
  width: number
  height: number
  inAtlas: boolean
}>

export type EngineTexture = FFI<{
  __size: 24
  __alignment: 8
  id: TextureId
  texturePath: string
  width: number
  height: number
  inAtlas: boolean
}>

export type FailedTexture = FFI<{
  __size: 24
  __alignment: 8
  texturePath: string
  failureReason: string
  id: TextureId
}>

export enum CreatePendingTextStatus {
  Success,
  OutputPendingTextNull,
}

export enum GetTextTypeByIdStatus {
  Success,
  TextAssetManagerNull,
  TextTypeNull,
  TextIdNotFound,
}

export enum GetTextByIdStatus {
  Success,
  TextAssetManagerNull,
  OutputTextNull,
  TextIdNotFound,
  TextTypeIncorrect,
}

export enum GetTextTypeByPathStatus {
  Success,
  TextPathNull,
  TextAssetManagerNull,
  TextTypeNull,
  TextPathNotFound,
}

export enum GetTextByPathStatus {
  Success,
  TextPathNull,
  TextAssetManagerNull,
  OutputTextNull,
  TextIdNotFound,
  TextTypeIncorrect,
}

export enum TextType {
  Pending,
  Engine,
  Loaded,
  Failed,
}

export enum LoadTextStatus {
  Success,
  TextAssetManagerNull,
  OutputPendingTextNull,
  LoadTextError,
}

export enum LoadTextByPendingTextStatus {
  Success,
  PendingTextNull,
  TextAssetManagerNull,
  LoadTextError,
}

export type PendingText = FFI<{
  __size: 16
  __alignment: 8
  textPath: string
  id: TextId
  setUpWatcher: boolean
}>

export type TextFormat = "json" | "toml" | "csv" | "text" | "unimplemented"

export type EngineText = FFI<{
  __size: 32
  __alignment: 8
  textPath: string
  format: TextFormat
  rawText: string
  id: TextId
}>

export type TextHash = [number, number, number, number, number, number, number, number]

export type LoadedText = FFI<{
  __size: 40
  __alignment: 8
  version: TextHash
  textPath: string
  format: TextFormat
  rawText: string
  id: TextId
  watcherSetUp: boolean
}>

export type FailedText = FFI<{
  __size: 24
  __alignment: 8
  failureReason: string
  textPath: string
  id: TextId
}>

declare const __brand: unique symbol
type Brand<T, TBrand extends symbol> = T & { [__brand]: TBrand }

const FiascoPointerBrand = Symbol("FiascoPointer")
export type FiascoPointer<T = unknown> = Brand<T, typeof FiascoPointerBrand>

const TextureIdBrand = Symbol("TextureId")
export type TextureId = Brand<number, typeof TextureIdBrand>

const TextIdBrand = Symbol("TextId")
export type TextId = Brand<number, typeof TextIdBrand>

export interface Engine {
  readonly POINTER_SIZE: 4 | 8

  // Memory Related
  malloc<T extends SizeAndAlignment | unknown>(
    size: T extends SizeAndAlignment ? T["__size"] : number,
    alignment: T extends SizeAndAlignment ? T["__alignment"] : number,
  ): FiascoPointer<T>
  free<T extends SizeAndAlignment | unknown>(
    ptr: FiascoPointer<T>,
    size: T extends SizeAndAlignment ? T["__size"] : number,
    alignment: T extends SizeAndAlignment ? T["__alignment"] : number,
  ): void
  debugAddress(ptr: FiascoPointer): bigint
  nullPointer(): FiascoPointer<null>
  getArrayBuffer(ptr: FiascoPointer, byteLength: number, offset: number): ArrayBuffer
  getString(ptr: FiascoPointer<string>, offset: number): string
  getPointer<T>(ptr: FiascoPointer, offset: number): FiascoPointer<T>
  setPointer(ptr: FiascoPointer, offset: number, data: FiascoPointer): void
  getUint8(ptr: FiascoPointer, offset: number): number
  setUint8(ptr: FiascoPointer, offset: number, value: number): void
  getUint16(ptr: FiascoPointer, offset: number): number
  setUint16(ptr: FiascoPointer, offset: number, value: number): void
  getUint32(ptr: FiascoPointer, offset: number): number
  setUint32(ptr: FiascoPointer, offset: number, value: number): void
  getInt8(ptr: FiascoPointer, offset: number): number
  setInt8(ptr: FiascoPointer, offset: number, value: number): void
  getInt16(ptr: FiascoPointer, offset: number): number
  setInt16(ptr: FiascoPointer, offset: number, value: number): void
  getInt32(ptr: FiascoPointer, offset: number): number
  setInt32(ptr: FiascoPointer, offset: number, value: number): void
  getFloat32(ptr: FiascoPointer, offset: number): number
  setFloat32(ptr: FiascoPointer, offset: number, value: number): void
  getFloat64(ptr: FiascoPointer, offset: number): number
  setFloat64(ptr: FiascoPointer, offset: number, value: number): void
  getBigInt64(ptr: FiascoPointer, offset: number): bigint
  setBigInt64(ptr: FiascoPointer, offset: number, value: bigint): void
  getBigUint64(ptr: FiascoPointer, offset: number): bigint
  setBigUint64(ptr: FiascoPointer, offset: number, value: bigint): void
  inputBufferPointer(): FiascoPointer
  inputBufferLength(): number

  // Engine API's
  spawn(components: FiascoPointer, componentsLength: number): bigint
  despawn(entityId: bigint): void
  loadScene(json: string): void
  addComponents(entityId: bigint, components: FiascoPointer, size: number): void
  removeComponents(entityId: bigint, componentIds: ArrayBuffer): void
  getEntityLabel(entityId: bigint): string | undefined
  setEntityLabel(entityId: bigint, label: string): void
  queryLen(queryPtr: FiascoPointer): number
  queryGet(queryPtr: FiascoPointer, index: number, componentPtrs: FiascoPointer): boolean
  queryGetEntity(queryPtr: FiascoPointer, entityId: bigint, componentPtrs: FiascoPointer): boolean
  queryGetLabel(queryPtr: FiascoPointer, label: string, componentPtrs: FiascoPointer): boolean
  setSystemEnabled(systemName: string, enabled: boolean): void
  eventCount(eventReaderPtr: FiascoPointer): number
  eventGet(eventReaderPtr: FiascoPointer, index: number): FiascoPointer
  eventSend(eventWriterPtr: FiascoPointer, bytes: ArrayBuffer): void
  setParent(entityId: bigint, parentId: bigint, keepWorldSpaceTransform: boolean): void
  clearParent(entityId: bigint, keepWorldSpaceTransform: boolean): void
  getParent(entityId: bigint): bigint | "no_parent" | "invalid_id"

  // Texture Asset Manager
  gpuInterfaceGetTextureAssetManagerMut(gpuInterface: FiascoPointer): FiascoPointer
  textureAssetManagerWhiteTextureId(): TextureId
  textureAssetManagerMissingTextureId(): TextureId
  textureAssetManagerRegisterNextTextureId(textureAssetManager: FiascoPointer): TextureId
  textureAssetManagerCreatePendingTexture(
    id: TextureId,
    assetPath: string,
    insertInAtlas: boolean,
    outputPendingTexture: FiascoPointer<PendingTexture>,
  ): CreatePendingTexture
  textureAssetManagerFreePendingTexture(pendingTexture: FiascoPointer<PendingTexture>): void
  textureAssetManagerFreeEngineTexture(engineTexture: FiascoPointer<EngineTexture>): void
  textureAssetManagerFreeLoadedTexture(loadedTexture: FiascoPointer<LoadedTexture>): void
  textureAssetManagerFreeFailedTexture(failedTexture: FiascoPointer<FailedTexture>): void
  textureAssetManagerGetTextureTypeById(
    textureAssetManager: FiascoPointer,
    textureId: TextureId,
    outTextureType: FiascoPointer<TextureType>,
  ): GetTextureTypeByIdStatus
  textureAssetManagerGetPendingTextureById(
    textureAssetManager: FiascoPointer,
    textureId: TextureId,
    outputTexture: FiascoPointer<PendingTexture>,
  ): GetTextureByIdStatus
  textureAssetManagerGetEngineTextureById(
    textureAssetManager: FiascoPointer,
    textureId: TextureId,
    outputTexture: FiascoPointer<EngineTexture>,
  ): GetTextureByIdStatus
  textureAssetManagerGetLoadedTextureById(
    textureAssetManager: FiascoPointer,
    textureId: TextureId,
    outputTexture: FiascoPointer<LoadedTexture>,
  ): GetTextureByIdStatus
  textureAssetManagerGetFailedTextureById(
    textureAssetManager: FiascoPointer,
    textureId: TextureId,
    outputTexture: FiascoPointer<FailedTexture>,
  ): GetTextureByIdStatus
  textureAssetManagerGetTextureTypeByPath(
    textureAssetManager: FiascoPointer,
    texturePath: string,
    outTextureType: FiascoPointer<TextureType>,
  ): GetTextureTypeByPathStatus
  textureAssetManagerGetPendingTextureByPath(
    textureAssetManager: FiascoPointer,
    texturePath: string,
    outputTexture: FiascoPointer<PendingTexture>,
  ): GetTextureByPathStatus
  textureAssetManagerGetEngineTextureByPath(
    textureAssetManager: FiascoPointer,
    texturePath: string,
    outputTexture: FiascoPointer<EngineTexture>,
  ): GetTextureByPathStatus
  textureAssetManagerGetLoadedTextureByPath(
    textureAssetManager: FiascoPointer,
    texturePath: string,
    outputTexture: FiascoPointer<LoadedTexture>,
  ): GetTextureByPathStatus
  textureAssetManagerGetFailedTextureByPath(
    textureAssetManager: FiascoPointer,
    texturePath: string,
    outputTexture: FiascoPointer<FailedTexture>,
  ): GetTextureByPathStatus
  textureAssetManagerAreIdsLoaded(textureAssetManager: FiascoPointer, ids: TextureId[]): boolean
  textureAssetManagerIsIdLoaded(textureAssetManager: FiascoPointer, id: TextureId): boolean
  textureAssetManagerLoadTexture(
    textureAssetManager: FiascoPointer,
    newTextureEventWriter: FiascoPointer,
    texturePath: string,
    insertInAtlas: boolean,
    outputPendingTexture: FiascoPointer<PendingTexture>,
  ): LoadTextureStatus
  textureAssetManagerLoadTextureByPendingTexture(
    textureAssetManager: FiascoPointer,
    newTextureEventWriter: FiascoPointer,
    outputPendingTexture: FiascoPointer<PendingTexture>,
  ): LoadTextureByPendingTextureStatus

  // Text Asset Manager
  textAssetManagerRegisterNextTextId(textAssetManager: FiascoPointer): TextId
  textAssetManagerCreatePendingText(
    id: TextId,
    assetPath: string,
    setUpWatcher: boolean,
    outputPendingText: FiascoPointer<PendingText>,
  ): CreatePendingTextStatus
  textAssetManagerFreePendingText(pendingText: FiascoPointer<PendingText>): void
  textAssetManagerFreeEngineText(engineText: FiascoPointer<EngineText>): void
  textAssetManagerFreeLoadedText(loadedText: FiascoPointer<LoadedText>): void
  textAssetManagerFreeFailedText(failedText: FiascoPointer<FailedText>): void
  textAssetManagerGetTextTypeById(
    textAssetManager: FiascoPointer,
    id: TextId,
    textType: FiascoPointer<TextType>,
  ): GetTextTypeByIdStatus
  textAssetManagerGetPendingTextById(
    textAssetManager: FiascoPointer,
    id: TextId,
    outText: FiascoPointer<PendingText>,
  ): GetTextByIdStatus
  textAssetManagerGetEngineTextById(
    textAssetManager: FiascoPointer,
    id: TextId,
    outText: FiascoPointer<EngineText>,
  ): GetTextByIdStatus
  textAssetManagerGetLoadedTextById(
    textAssetManager: FiascoPointer,
    id: TextId,
    outText: FiascoPointer<LoadedText>,
  ): GetTextByIdStatus
  textAssetManagerGetFailedTextById(
    textAssetManager: FiascoPointer,
    id: TextId,
    outText: FiascoPointer<FailedText>,
  ): GetTextByIdStatus
  textAssetManagerGetTextTypeByPath(
    textAssetManager: FiascoPointer,
    path: string,
    textType: FiascoPointer<TextType>,
  ): GetTextTypeByPathStatus
  textAssetManagerGetPendingTextByPath(
    textAssetManager: FiascoPointer,
    path: string,
    outText: FiascoPointer<PendingText>,
  ): GetTextByPathStatus
  textAssetManagerGetEngineTextByPath(
    textAssetManager: FiascoPointer,
    path: string,
    outText: FiascoPointer<EngineText>,
  ): GetTextByPathStatus
  textAssetManagerGetLoadedTextByPath(
    textAssetManager: FiascoPointer,
    path: string,
    outText: FiascoPointer<LoadedText>,
  ): GetTextByPathStatus
  textAssetManagerGetFailedTextByPath(
    textAssetManager: FiascoPointer,
    path: string,
    outText: FiascoPointer<FailedText>,
  ): GetTextByPathStatus
  textAssetManagerAreIdsLoaded(textAssetManager: FiascoPointer, ids: TextId[]): boolean
  textAssetManagerIsIdLoaded(textAssetManager: FiascoPointer, id: TextId): boolean
  textAssetManagerLoadText(
    textAssetManager: FiascoPointer,
    newTextEventWriter: FiascoPointer,
    path: string,
    setUpWatcher: boolean,
    outPendingText: FiascoPointer<PendingText>,
  ): LoadTextStatus
  textAssetManagerLoadTextByPendingText(
    textAssetManager: FiascoPointer,
    newTextEventWriter: FiascoPointer,
    pendingText: FiascoPointer<PendingText>,
  ): LoadTextByPendingTextStatus
}

export interface EcsModule {
  /**
   * The version of the engine that this module is compatible with.
   */
  voidTargetVersion(): number

  /**
   * The name of this ecs module.
   */
  moduleName(): string

  /**
   * Called once during module startup.
   */
  init(): void

  /**
   * Called once right before this module is unloaded
   */
  deinit(): void

  /**
   * Called with the internal engine instance.
   */
  setEngine(engine: Engine): void

  /**
   * Called infinitly until `undefined` is returned.
   * For every index provided, return an associated string id.
   * @param index The component index
   */
  componentStringId(index: number): string | undefined

  /**
   * Tells the module which numerical component id has been assigned to each component.
   */
  setComponentId(stringId: string, componentId: number): void

  /**
   * For each resource which this module defines, the engine will allocate the requested size
   * and pass a pointer to that memory, for the module to initialize the resource at startup.
   */
  resourceInit(stringId: string, ptr: FiascoPointer): number

  /**
   * When restoring a state snapshot, for each resource which this module
   * defines, the engine will pass a pointer to the resource for the module to
   * deserialize the resource state into, from a previously serialized buffer.
   *
   * The binary buffer will typically be a JSON representation of the resource.
   */
  resourceDeserialize(stringId: string, ptr: FiascoPointer, serialized: ArrayBuffer): number

  /**
   * When taking a state snapshot, for each resource which this module defines,
   * the engine will pass a pointer to the resource for the module to
   * serialize the resource state from, into a binary buffer.
   *
   * The binary buffer will typically be a JSON representation of the resource, but not limited to that.
   */
  resourceSerialize(stringId: string, ptr: FiascoPointer): ArrayBuffer

  /**
   *
   * When deserializing scene json, the engine will pass a pointer for the module to deserialize the component from
   * the jsonString parameter
   */
  componentDeserializeJson(stringId: string, ptr: FiascoPointer, json: string): number

  /**
   * Returns the component size in bytes, for each component returned by `componentStringId`.
   * Called for both components and resources.
   */
  componentSize(stringId: string): number

  /**
   * Returns the component alignment in bytes, for each component returned by `componentStringId`.
   * Called for both components and resources.
   */
  componentAlign(stringId: string): number

  /**
   * Returns the `EcsType` for this string id, for each component returned by `componentStringId`.
   */
  componentType(stringId: string): EcsType

  /**
   * Returns the number of systems in this module.
   */
  systemsLen(): number

  /**
   * Returns the name of a system.
   */
  systemName(systemIndex: number): string

  /**
   * Returns whether a system is a "once" system, i.e. it runs only once at startup.
   * Most systems are not, and should run once per frame.
   */
  systemIsOnce(systemIndex: number): boolean

  /**
   * Returns the function for each system.
   *
   * System functions take a `ptr` parameter, which is a double pointer to system inputs.
   * System functions return zero on success, non-zero on error.
   */
  systemFunction(systemIndex: number): (ptr: FiascoPointer) => number

  /**
   * Returns the number of inputs for each system.
   */
  systemArgsLen(systemIndex: number): number

  /**
   * Returns the type of input of a given system argument.
   */
  systemArgType(systemIndex: number, argIndex: number): ArgType

  /**
   * Returns the resource string id for any resource system inputs.
   */
  systemArgComponent(systemIndex: number, argIndex: number): string

  /**
   * Returns the event string id for any `ArgType.EventReader`/`ArgType.EventWriter` system inputs.
   */
  systemArgEvent(systemIndex: number, argIndex: number): string

  /**
   * Returns the number of inputs to each `Query`.
   */
  systemQueryArgsLen(systemIndex: number, argIndex: number): number

  /**
   * Returns the type of input for each `Query` input.
   */
  systemQueryArgType(systemIndex: number, argIndex: number, queryArgIndex: number): ArgType

  /**
   * Returns the component string id each `Query` input.
   */
  systemQueryArgComponent(systemIndex: number, argIndex: number, queryArgIndex: number): string
}
