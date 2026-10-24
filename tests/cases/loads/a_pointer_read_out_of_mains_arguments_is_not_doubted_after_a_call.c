void release_all(void);

int main(int argc, int **argv) {
    if (argc < 1) {
        return 0;
    }
    if (argv == 0) {
        return 0;
    }
    int *a = *argv;
    if (a == 0) {
        return 0;
    }
    release_all();
    return *a;
}
