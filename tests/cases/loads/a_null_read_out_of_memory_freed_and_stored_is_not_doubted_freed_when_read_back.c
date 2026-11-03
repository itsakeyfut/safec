void *malloc(int n);
void free(void *p);
int f(int c) {
    int **box = malloc(8);
    if (!box) return 0;
    int **cell = malloc(8);
    if (!cell) return 0;
    *cell = 0;
    int *p = *cell;
    free(p);             /* free(NULL): defined */
    *box = p;
    int *r = *box;
    int v = 0;
    if (r) v = *r;       /* r is null: not executed */
    free(cell);
    free(box);
    return v;
}
