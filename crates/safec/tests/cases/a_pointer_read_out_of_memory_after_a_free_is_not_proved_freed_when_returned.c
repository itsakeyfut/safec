void *malloc(int n);
void free(void *p);

int *get(void) {
    int **tab = malloc(8);
    if (tab == 0) {
        return 0;
    }
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    *tab = p;
    free(p);
    int *q = *tab;
    return q;
}
