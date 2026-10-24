void *malloc(int n);
void keep(int **pp);
int use2(int **pp);

int main(void) {
    int **t = malloc(8);
    if (t == 0) {
        return 0;
    }
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    keep(&a);
    *t = a;
    return use2(t);
}
