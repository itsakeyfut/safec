void *malloc(int n);
void release_in(int **d);
int use2(int **pp);

int main(void) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    int **d = malloc(8);
    if (d == 0) {
        return 0;
    }
    *d = a;
    release_in(d);
    return use2(&a);
}
