void *malloc(int n);
void free(void *p);
void release(int *p);
int cond(void);

int *get(void) {
    int **tab = malloc(8);
    if (tab == 0) {
        return 0;
    }
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    p[0] = 1;
    *tab = p;
    free(p);
    if (cond()) {
        free(tab);
    }
    return *tab;
}
