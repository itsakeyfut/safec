void *malloc(int n);
void free(void *p);

int main(void) {
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
    *tab = malloc(4);
    int *q = *tab;
    if (q == 0) {
        return 0;
    }
    q[0] = 1;
    return *q;
}
