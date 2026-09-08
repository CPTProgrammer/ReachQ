import { mount } from 'svelte';
import StreamPreview from './StreamPreview.svelte';
import './preview.css';

mount(StreamPreview, { target: document.getElementById('app')! });
