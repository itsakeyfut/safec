void *malloc(int n);
void free(void *p);

int main(void) {
    int **tab = malloc(16);
    if (tab == 0) {
        return 0;
    }
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    p[0] = 1;
    tab[0] = p;
    tab[1] = 0;
    int *q = tab[1];
    free(q);
    return *p;
}
