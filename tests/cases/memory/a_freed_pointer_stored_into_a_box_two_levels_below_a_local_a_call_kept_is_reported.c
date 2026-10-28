void *malloc(int n);
void free(void *p);
void keepp(int ****p);
void use_kept(void);
void make(void) {
    int ***slot;
    keepp(&slot);
    int ***c = malloc(8);
    if (c == 0) { return; }
    int **b = malloc(8);
    if (b == 0) { return; }
    int *x = malloc(4);
    if (x == 0) { return; }
    *c = b;
    slot = c;
    free(x);
    *b = x;
    use_kept();
}
