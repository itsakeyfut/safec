void *memcpy(void *d, void *s, int n);
void f(int **out, int c) {
    int *m[1];
    if (!out) return;
    for (int i = 0; i < 2; i++) { if (c) memcpy(out, m, 8); m[0] = 0; }
}
