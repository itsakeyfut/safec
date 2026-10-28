void *malloc(int n);
void free(void *p);
void keepp(int ***p);
void use_kept(void);
void make(void) {
    int *t;
    int **slot;
    keepp(&slot);
    int *x = malloc(4);
    if (x == 0) { return; }
    slot = &t;
    int **pt = &t;
    free(x);
    *pt = x;
    use_kept();
}
