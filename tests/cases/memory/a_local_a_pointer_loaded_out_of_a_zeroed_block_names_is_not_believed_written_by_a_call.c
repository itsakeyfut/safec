void *malloc(int n);
void *memset(void *d, int c, int n);
void fill(int ***b);
int f(void) {
    int *a;
    int ***w = malloc(8);
    if (!w) return 0;
    *w = &a;
    memset(w, 0, 8);
    int ***b = malloc(8);
    if (!b) return 0;
    *b = *w;
    fill(b);
    return a != 0;
}
