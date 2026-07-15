import { Macro } from '../src/macro';

const macro = new Macro({ env: 'dev' });

const doc = await macro.documents.create({ name: 'foo' });
console.log('created document:', doc.id);

const fav = await doc.favorite();
console.log('favorited:', fav.id);
