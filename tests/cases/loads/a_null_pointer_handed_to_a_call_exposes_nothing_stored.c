void *malloc(int n);
void log_ptr(int *p);

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
    int *z = 0;
    log_ptr(z);
    return *p;
}
