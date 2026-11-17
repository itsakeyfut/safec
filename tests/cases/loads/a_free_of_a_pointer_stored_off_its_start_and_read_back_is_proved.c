void *malloc(int n);
void free(void *p);
int f(void) {
    int *m = malloc(8);
    if (m == 0) return 0;
    int **box = malloc(8);
    if (box == 0) {
        free(m);
        return 0;
    }
    *box = m + 1;
    int *t = *box;
    free(t);
    free(box);
    return 0;
}
