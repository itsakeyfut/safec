void *malloc(int n);
int keep(int *p);
int release(int **t);

int main(void) {
    int r;
    int **tab = malloc(8);
    int *a = malloc(4);
    if (tab == 0) {
        return 0;
    }
    if (a == 0) {
        return 0;
    }
    *tab = a;
    a[0] = 1;
    r = keep(a) + release(tab);
    return r;
}
