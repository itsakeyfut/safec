void *malloc(int n);
void drop_inner(int **t);

int main(void) {
    int ***holder = malloc(8);
    if (holder == 0) {
        return 0;
    }
    int **tab = malloc(8);
    if (tab == 0) {
        return 0;
    }
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    *holder = tab;
    *tab = p;
    int **t = *holder;
    drop_inner(t);
    return *p;
}
