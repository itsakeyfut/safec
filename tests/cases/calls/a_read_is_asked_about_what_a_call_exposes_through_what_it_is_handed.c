void *malloc(int n);
void *memcpy(void *d, void *s, int n);
void **slot(void);
int release_all(void);

int main(void) {
    void **out_ = slot();
    int x;
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
    r = (x = a[0]) + (memcpy(out_, &tab, 8) != 0) + release_all();
    return r;
}
