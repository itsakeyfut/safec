void *malloc(int n);
void show(int *p);

int f(int c) {
    int ***holder = malloc(8);
    if (holder == 0) {
        return 0;
    }
    int **tab = malloc(8);
    if (tab == 0) {
        return 0;
    }
    int **other = malloc(8);
    if (other == 0) {
        return 0;
    }
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    *holder = tab;
    *tab = p;
    int **t = other;
    if (c) {
        t = *holder;
    }
    if (t == 0) {
        return 0;
    }
    show(*t);
    return *p;
}
