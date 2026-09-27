void *malloc(int n);
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
    *tab = p;
    int *q = *tab;
    q = 0;
    show(q);
    return *p;
}
