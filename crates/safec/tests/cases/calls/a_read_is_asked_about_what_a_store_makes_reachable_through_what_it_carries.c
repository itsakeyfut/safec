void *malloc(int n);
int keep(int ***box);
int release_all(void);

int main(void) {
    int x;
    int r;
    int ***box = malloc(8);
    int **tab = malloc(8);
    int *a = malloc(4);
    if (box == 0) {
        return 0;
    }
    if (tab == 0) {
        return 0;
    }
    if (a == 0) {
        return 0;
    }
    keep(box);
    *tab = a;
    a[0] = 1;
    r = (x = a[0]) + (*box = tab, 0) + release_all();
    return r;
}
