void *malloc(int n);
void free(void *p);
int use2(int **pp);

int main(int c) {
    int **t = malloc(8);
    if (t == 0) {
        return 0;
    }
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    *t = a;
    if (c) {
        free(a);
    }
    return use2(t);
}
