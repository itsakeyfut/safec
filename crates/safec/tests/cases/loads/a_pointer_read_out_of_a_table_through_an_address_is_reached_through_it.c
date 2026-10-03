void *malloc(int n);
void drop_slot(int **pp);

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
    int *slot = 0;
    int **pp = &slot;
    *pp = *tab;
    drop_slot(pp);
    return *p;
}
