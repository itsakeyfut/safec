void *malloc(int n);
void free(void *p);

int *first(void) {
    int **tab = malloc(8);
    if (tab == 0) {
        return 0;
    }
    free(tab);
    return *tab;
}
