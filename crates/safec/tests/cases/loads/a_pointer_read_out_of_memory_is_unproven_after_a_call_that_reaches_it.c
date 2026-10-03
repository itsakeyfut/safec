void *malloc(int n);
void release(int **t);

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
    int *q = *tab;
    release(tab);
    if (q == 0) {
        return 0;
    }
    return *q;
}
