void *malloc(unsigned long long size);
void free(void *pointer);

struct Flags {
    unsigned ready : 1;
    int delta : 4;
    int *pointer;
};

int written_then_read(void) {
    struct Flags f;
    f.ready = 1;
    f.delta = -1;
    return f.ready + f.delta;
}

int beside_a_pointer(void) {
    int x = 1;
    struct Flags f;
    f.pointer = &x;
    f.ready = 0;
    return *f.pointer;
}

int through_a_freed_pointer(void) {
    struct Flags *f = malloc(sizeof(struct Flags));
    if (!f) {
        return 1;
    }
    f->ready = 1;
    free(f);
    return f->ready;
}

int main(void) {
    return written_then_read() + beside_a_pointer() + through_a_freed_pointer();
}
