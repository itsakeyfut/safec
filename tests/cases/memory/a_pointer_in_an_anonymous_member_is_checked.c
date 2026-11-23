void *malloc(unsigned long long size);
void free(void *pointer);

struct Holder {
    int kind;
    union {
        int *pointer;
        long long number;
    };
};

int written_then_read(void) {
    int x = 1;
    struct Holder h;
    h.kind = 0;
    h.pointer = &x;
    return *h.pointer - 1;
}

int number_read_as_pointer(void) {
    struct Holder h;
    h.kind = 0;
    h.number = 42;
    return *h.pointer;
}

int freed_then_read(void) {
    struct Holder h;
    h.kind = 0;
    h.pointer = malloc(sizeof(int));
    if (!h.pointer) {
        return 1;
    }
    *h.pointer = 0;
    free(h.pointer);
    return *h.pointer;
}

int main(void) {
    return written_then_read() + number_read_as_pointer() + freed_then_read();
}
