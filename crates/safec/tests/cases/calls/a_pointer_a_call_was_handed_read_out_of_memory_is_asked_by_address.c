void *malloc(int n);
void release(int *p);
int use2(int **pp);

int main(void) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    int **t = malloc(8);
    if (t == 0) {
        return 0;
    }
    *t = a;
    release(*t);
    return use2(&a);
}
