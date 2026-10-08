void free(void *p);
void *malloc(int n);
int release(int *q);

int f(int c) {
    int **tab = malloc(8);
    if (!tab) {
        return 0;
    }
    int *q = malloc(4);
    *tab = q;
    if (c) {
        free(tab);
    }
    int x = release(*tab) + (free(q), 0);
    return x;
}
