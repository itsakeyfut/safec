void *malloc(int n);
void free(void *p);

int main(void) {
    int **tab = malloc(16);
    if (tab == 0) {
        return 0;
    }
    int *p = malloc(4);
    int *r = malloc(4);
    if (p == 0) {
        return 0;
    }
    if (r == 0) {
        return 0;
    }
    r[0] = 1;
    tab[0] = p;
    tab[1] = r;
    free(p);
    int *q = tab[1];
    if (q == 0) {
        return 0;
    }
    return *q;
}
