void *malloc(int n);
void log_int(int v);
void show(int *p);

int main(void) {
    int **tab = malloc(8);
    if (tab == 0) {
        return 0;
    }
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    int *w = malloc(8);
    if (w == 0) {
        return 0;
    }
    *p = 0;
    *tab = p;
    int x = *p;
    log_int(x);
    int *r = w + *p;
    show(r);
    return *p;
}
