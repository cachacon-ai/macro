import { isNativeMobilePlatform } from '@core/mobile/isNativeMobilePlatform';
import { createSyntheticFileEntry } from '@core/util/dataTransfer';
import { isAndroid, isIOS } from '@solid-primitives/platform';
import { invoke, isTauri } from '@tauri-apps/api/core';
import {
  createNativeStagedUploadFile,
  type NativeStagedUploadData,
} from './nativeStagedUpload';

const IMAGE_FILE_EXTENSION =
  /\.(png|jpe?g|gif|webp|bmp|tiff?|heic|heif|avif)$/i;

function isMobileDropContext(): boolean {
  return isTauri() && (isIOS || isAndroid || isNativeMobilePlatform());
}

function isImageFile(file: File): boolean {
  if (file.type.startsWith('image/')) return true;
  // iOS can surface HEIC/HEIF drops with an empty MIME type.
  if (file.type === '') return IMAGE_FILE_EXTENSION.test(file.name);
  return false;
}

function fileToBase64(file: File): Promise<string> {
  return file.arrayBuffer().then((buffer) => {
    const bytes = new Uint8Array(buffer);
    let binary = '';
    const chunkSize = 0x8000;
    for (let offset = 0; offset < bytes.length; offset += chunkSize) {
      binary += String.fromCharCode(
        ...bytes.subarray(offset, offset + chunkSize)
      );
    }
    return btoa(binary);
  });
}

function entryToFile(entry: FileSystemFileEntry): Promise<File | null> {
  return new Promise((resolve) => {
    try {
      entry.file(
        (file) => resolve(file),
        () => resolve(null)
      );
    } catch {
      resolve(null);
    }
  });
}

async function stageDroppedImage(file: File): Promise<File | null> {
  try {
    const imageData = await fileToBase64(file);
    const staged = await invoke<NativeStagedUploadData>(
      'plugin:pasteboard|stage_dropped_image',
      { imageData, fileName: file.name }
    );
    return createNativeStagedUploadFile('pasteboard', staged);
  } catch {
    return null;
  }
}

async function processEntry(
  entry: FileSystemFileEntry
): Promise<FileSystemFileEntry> {
  const file = await entryToFile(entry);
  if (!file || !isImageFile(file)) return entry;

  const staged = await stageDroppedImage(file);
  return staged ? createSyntheticFileEntry(staged) : entry;
}

/**
 * Runs dragged image entries through the native pasteboard plugin so they get
 * the same downscaling and re-encoding as pasted images.
 *
 * Needed on mobile WKWebView where the browser hands us the raw drop bytes
 * (often large or HEIC), which the upload pipeline can't process on its own.
 * Non-image entries and any entries that fail native staging pass through
 * unchanged.
 */
export async function processMobileDroppedImageEntries(
  fileEntries: FileSystemFileEntry[]
): Promise<FileSystemFileEntry[]> {
  if (!isMobileDropContext() || fileEntries.length === 0) {
    return fileEntries;
  }
  return Promise.all(fileEntries.map(processEntry));
}
