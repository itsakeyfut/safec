// A call handed `b` after `*b = 0;` is handed nothing freed (#656).
void *malloc(int n);
void free(void *p);
void give(int **b);
int f(void) {
    int **b = malloc(8);
    if (b == 0) { return 0; }
    int *x = malloc(4);
    *b = x;
    free(x);
    *b = 0;
    give(b);
    return 0;
}
