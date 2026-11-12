// `b[k] = 0;` may land past the end of `b` rather than over the freed `x`
// it holds, so `b` is still doubted where it is returned (#656, ADR-0094
// rule 1).
void *malloc(int n);
void free(void *p);
int **make(int k) {
    int **b = malloc(8);
    if (b == 0) { return 0; }
    int *x = malloc(4);
    *b = x;
    free(x);
    b[k] = 0;
    return b;
}
