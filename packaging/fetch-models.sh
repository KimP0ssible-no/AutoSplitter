#!/bin/sh
# Download the RapidOCR models into models/ and check their SHA-256 hashes.
# Files already there with the right hash are kept.
#
#   packaging/fetch-models.sh
set -eu
base=https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.8.0
mkdir -p models
while read -r hash path; do
    file=models/$(basename "$path")
    if [ -f "$file" ] && echo "$hash  $file" | sha256sum -c --status; then
        continue
    fi
    curl -sSfL --retry 3 -o "$file.part" "$base/$path"
    echo "$hash  $file.part" | sha256sum -c --quiet
    mv "$file.part" "$file"
done <<'LIST'
ea07c15d38ac40cd69da3c493444ec75b44ff23840553ff8ba102c1219ed39c2 onnx/PP-OCRv4/det/en_PP-OCRv3_det_mobile.onnx
c3461add59bb4323ecba96a492ab75e06dda42467c9e3d0c18db5d1d21924be8 onnx/PP-OCRv5/rec/en_PP-OCRv5_rec_mobile.onnx
e025a66d31f327ba0c232e03f407ae8d105e1e709e7ccb3f408aa778c24e70d6 paddle/PP-OCRv5/rec/en_PP-OCRv5_rec_mobile/ppocrv5_en_dict.txt
4d97c44a20d30a81aad087d6a396b08f786c4635742afc391f6621f5c6ae78ae onnx/PP-OCRv5/det/ch_PP-OCRv5_det_mobile.onnx
5825fc7ebf84ae7a412be049820b4d86d77620f204a041697b0494669b1742c5 onnx/PP-OCRv5/rec/ch_PP-OCRv5_rec_mobile.onnx
d1979e9f794c464c0d2e0b70a7fe14dd978e9dc644c0e71f14158cdf8342af1b paddle/PP-OCRv5/rec/ch_PP-OCRv5_rec_mobile/ppocrv5_dict.txt
LIST
